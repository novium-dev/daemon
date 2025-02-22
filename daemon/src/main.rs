#![warn(clippy::string_to_string)]
#![cfg_attr(feature = "trace_sync", deny(clippy::disallowed_methods))]
#![cfg_attr(not(feature = "trace_sync"), allow(clippy::disallowed_methods))]

use crate::sync::broadcast::Sender;
use crate::sync::Mutex;
use crate::sync::RwLock;
use axum::extract::Request;
use bollard::network::CreateNetworkOptions;
use bollard::Docker;
use libc::SIGINT;
use reqwest::Client;
use ringbuffer::AllocRingBuffer;
#[cfg(feature = "ssh")]
use russh::server::{Config, Server as SSHServer};
use russh::{MethodKind, MethodSet, SshId};
use signal_hook::iterator::Signals;
use sqlx::mysql::MySqlPoolOptions;
use sqlx::{query, MySql, Pool};
use std::collections::HashMap;
use std::env;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::process::exit;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWrite};
use tokio::runtime::Handle;
use tokio::task::JoinSet;
use tower_http::trace::TraceLayer;
use tower_http::validate_request::ValidateRequestHeaderLayer;
use tracing::{error, info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use uuid::Uuid;

pub use daemon_common as common;

use crate::config::DaemonConfig;
use crate::server::watcher::attach;
use crate::server::FetchedServer;
use crate::storage::sync::{storage_read, StorageLockData};
use crate::storage::StorageIdentifier;
use crate::terminal::TerminalSession;
use crate::utils::ChannelWriteData;

mod config;
#[cfg(feature = "database")]
mod database;
mod server;

mod files;
mod sql;
#[cfg(feature = "ssh")]
mod ssh;
mod storage;

mod ctlsock;
mod network;
pub(crate) mod setup;
mod sync;
mod terminal;
mod utils;
mod web;

#[derive(OpenApi)]
#[openapi()]
struct ApiDoc;

const GIT_HASH: &str = env!("GIT_HASH");

#[tokio::main]
async fn main() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .unwrap();

    let reg = tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::from_level(
            if let Ok(level) = env::var("DEBUG") {
                if level.eq("1") {
                    tracing::Level::DEBUG
                } else {
                    tracing::Level::INFO
                }
            } else if cfg!(debug_assertions) {
                tracing::Level::DEBUG
            } else {
                tracing::Level::INFO
            },
        ))
        .with(tracing_subscriber::fmt::layer());

    #[cfg(not(all(feature = "sentry", not(debug_assertions))))]
    reg.init();

    #[cfg(all(feature = "sentry", not(debug_assertions)))]
    {
        reg.with(sentry::integrations::tracing::layer()).init();
        let sentry = sentry::init(("https://79a1f560730aef34fdc4e4a3f3d9ce92@o4507044844404736.ingest.us.sentry.io/4508313795297280", sentry::ClientOptions {
            release: sentry::release_name!(),
            ..Default::default()
        }));
        if sentry.is_enabled() {
            info!("Enabled Sentry integration");
            std::env::set_var("RUST_BACKTRACE", "1");
        }
    }

    if env::args().nth(1).is_some_and(|s| s.eq("setup")) {
        let _ = tokio::task::spawn_blocking(setup::setup).await;
        return;
    }

    let mut config_buffer = String::new();
    File::open("config.toml")
        .await
        .inspect_err(|_err| error!("Failed to open config file"))
        .unwrap()
        .read_to_string(&mut config_buffer)
        .await
        .unwrap();
    let config: DaemonConfig = toml::from_str(&config_buffer)
        .inspect_err(|_err| error!("Failed to read config file"))
        .unwrap();

    info!("Starting Daemon on version {}", GIT_HASH);

    let servers = HashMap::new();

    info!("Connecting to Docker...");
    let docker = bollard::Docker::connect_with_local_defaults()
        .inspect_err(|_err| error!("Failed to connect to docker"))
        .unwrap();

    match docker
        .inspect_network::<String>(crate::common::NETWORK_DEFAULT, None)
        .await
    {
        Ok(_) => {}
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => {
            docker
                .create_network(CreateNetworkOptions {
                    name: crate::common::NETWORK_DEFAULT.to_string(),
                    options: HashMap::from([
                        (
                            "com.docker.network.bridge.enable_icc".to_string(),
                            false.to_string(),
                        ),
                        (
                            "com.docker.network.bridge.enable_ip_masquerade".to_string(),
                            false.to_string(),
                        ),
                    ]),
                    internal: false,
                    ..CreateNetworkOptions::default()
                })
                .await
                .unwrap();
        }
        Err(err) => {
            panic!("Failed to inspect docker network: {:?}", err);
        }
    }
    match docker
        .inspect_network::<String>(crate::common::NETWORK_INTERNET, None)
        .await
    {
        Ok(_) => {}
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        }) => {
            docker
                .create_network(CreateNetworkOptions {
                    name: crate::common::NETWORK_INTERNET.to_string(),
                    options: HashMap::from([
                        (
                            "com.docker.network.bridge.enable_icc".to_string(),
                            false.to_string(),
                        ),
                        (
                            "com.docker.network.bridge.enable_ip_masquerade".to_string(),
                            true.to_string(),
                        ),
                    ]),
                    internal: false,
                    ..CreateNetworkOptions::default()
                })
                .await
                .unwrap();
        }
        Err(err) => {
            panic!("Failed to inspect docker network: {:?}", err);
        }
    }

    info!("Establishing Database Connection...");
    let conn = {
        loop {
            if let Ok(conn) = MySqlPoolOptions::new()
                .max_connections(5)
                .connect(&config.database)
                .await
            {
                break conn;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    };

    let config = Arc::new(config);
    let state = DaemonState {
        config: config.clone(),
        servers: Arc::new(Mutex::new(servers)),
        storages: Arc::new(Mutex::new(HashMap::new())),
        docker: Arc::new(docker),
        conn: Arc::new(conn),
        client: Arc::new(Client::default()),
    };
    sql::init(&state.conn)
        .await
        .inspect_err(|err| error!("Failed to initialize sql: {:?}", err))
        .unwrap();

    if let Ok(containers) = state
        .docker
        .list_containers::<String>(None)
        .await
        .inspect_err(|err| {
            warn!(
                "Failed to scan for running containers... (assuming there are none): {:?}",
                err
            )
        })
    {
        let mut servers = state.servers.lock().await;

        let mut restored = 0;
        for container in containers {
            let names = container.names.unwrap_or_default();
            let mut id = None::<u32>;
            for mut name in names {
                if let Some(suffix) = name.strip_prefix("/") {
                    name = suffix.to_string();
                }
                if let Some(suffix) = name.strip_prefix(crate::common::CONTAINER_PREFIX) {
                    if let Ok(container_id) = suffix.parse() {
                        id = Some(container_id);
                        break;
                    }
                }
            }
            if let Some(id) = id {
                servers.insert(id, Arc::default());
                let server_data_arc = servers.get(&id).unwrap().clone();

                match query!(
                    "SELECT `cpu`, `ram`, `swap` FROM `servers` WHERE `id` = ?",
                    id
                )
                .fetch_optional(state.conn.deref())
                .await
                .unwrap()
                {
                    Some(row) => {
                        let mut server_data = server_data_arc.lock().await;
                        let StorageLockData {
                            lock: storage_lock,
                            value: storage_value,
                            stats_lock: storage_stats_lock,
                            ..
                        } = storage_read(
                            &state.config,
                            state.conn.deref(),
                            &state.storages,
                            StorageIdentifier::Primary(id),
                            false,
                        )
                        .await
                        .unwrap();

                        let cpu_max = row.cpu.map(|cpu| cpu * 100f32).unwrap_or(-1f32);
                        let ram_max = row.ram.map(|ram| ram as i64).unwrap_or(-1);
                        let swap_max = row.swap.map(|swap| swap as i64).unwrap_or(-1);

                        if let Err(err) = attach(
                            id,
                            &state,
                            cpu_max,
                            ram_max,
                            swap_max,
                            server_data.deref_mut(),
                            server_data_arc.clone(),
                            storage_lock,
                            storage_value,
                            storage_stats_lock,
                        )
                        .await
                        {
                            warn!("Failed to reattach server {}: {:?}", id, err);
                        } else {
                            restored += 1;
                        }
                    }
                    None => {
                        warn!(
                            "Found server that is missing in database: {}. Killing...",
                            id
                        );
                        let _ = state
                            .docker
                            .kill_container::<String>(&FetchedServer::container_name(id), None)
                            .await;
                    }
                }
            }
        }

        if restored > 0 {
            info!("Successfully reattached to {} container", restored);
        }
    }

    info!("Starting Webserver...");

    #[allow(unused_mut)]
    let mut router = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(web::config())
        .route_layer(ValidateRequestHeaderLayer::bearer(&config.token))
        .merge(web::config_unauthorized());

    #[cfg(feature = "user")]
    {
        let cors = tower_http::cors::CorsLayer::new()
            .allow_origin(tower_http::cors::AllowOrigin::exact(
                axum::http::header::HeaderValue::from_str(&config.origin).unwrap(),
            ))
            .allow_methods([axum::http::Method::GET])
            .allow_headers(tower_http::cors::AllowHeaders::any());

        router = router.merge(web::user_config().layer(cors));
    }

    #[allow(unused_variables, unused_mut)]
    let (mut router, api) = router.split_for_parts();

    #[cfg(debug_assertions)]
    {
        use utoipa_swagger_ui::SwaggerUi;
        router = router.merge(SwaggerUi::new("/docs").url("/docs/openapi.json", api));
    }

    let mut app = router
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    #[cfg(feature = "sentry")]
    {
        use sentry::integrations::tower::{NewSentryLayer, SentryHttpLayer};
        app = app
            .layer(NewSentryLayer::<Request>::new_from_top())
            .layer(SentryHttpLayer::with_transaction());
    }

    let state_cloned = state.clone();
    let state = Arc::new(state);

    #[cfg(feature = "ssh")]
    {
        let state1 = state.clone();
        let config1 = config.clone();
        tokio::task::spawn(async move {
            let mut keys = Vec::with_capacity(config1.ssh.host_key.len());
            for key in &config1.ssh.host_key {
                keys.push(match key.as_russh_key().await {
                    Ok(key) => key,
                    Err(err) => {
                        error!("Failed to start ssh server (key): {:?}", err);
                        exit(-1);
                    }
                });
                tracing::debug!("Loaded ssh key: {:?}", key);
            }
            let mut server = crate::ssh::server::DaemonSSHServer {
                state: state1,
                cache_invalidation: Duration::from_millis(config1.ssh.auth_cache_duration),
            };
            let mut methods = MethodSet::empty();
            methods.push(MethodKind::Password);
            if let Err(err) = server
                .run_on_address(
                    Arc::new(Config {
                        keys,
                        server_id: SshId::Standard(format!(
                            "{}{}",
                            common::SSH_BANNER_PREFIX,
                            GIT_HASH
                        )),
                        methods,
                        ..Config::default()
                    }),
                    &config1.ssh.bind,
                )
                .await
            {
                error!("Failed to start ssh server (bind): {:?}", err);
                exit(-1);
            }
        });
    }

    storage::worker::run(state_cloned.clone()).await;

    tokio::task::spawn(ctlsock::run(state_cloned));

    let state1 = state.clone();
    let mut signal = Signals::new([SIGINT]).unwrap();
    tokio::task::spawn_blocking(move || {
        if signal.forever().next().is_some() {
            info!("Interrupt received. Shutting down all servers...");
            Handle::current().block_on(async move {
                let servers = state1.servers.lock().await;
                let mut set = JoinSet::new();
                for (id, server) in servers.deref() {
                    let server = server.clone().lock_owned().await;
                    let docker = state1.docker.clone();
                    let id = *id;
                    set.spawn(async move {
                        let _ = FetchedServer {
                            id,
                            ..FetchedServer::default()
                        }
                        .stop(&docker, server)
                        .await;
                    });
                }
                while set.join_next().await.is_some() {}
                info!("Done");
                exit(0);
            });
        }
    });

    let state1 = state.clone();

    let deadlock_detection = if let Ok(level) = env::var("DEADLOCK_DETECTION") {
        level.eq("1")
    } else {
        cfg!(debug_assertions)
    };
    if deadlock_detection {
        info!("Automatic Deadlock detection is enabled.");
        tokio::task::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                let servers =
                    tokio::time::timeout(Duration::from_secs(10), state1.servers.lock()).await;
                match servers {
                    Ok(servers) => {
                        let servers_cloned = servers.clone();
                        drop(servers);
                        for (id, server_data_mutex) in servers_cloned {
                            let server = tokio::time::timeout(
                                Duration::from_secs(10),
                                server_data_mutex.lock(),
                            )
                            .await;
                            match server {
                                Ok(server) => {
                                    let sessions = server.sessions.clone();
                                    let attach_write = server.attach_write.clone();
                                    let buffer = server.buffer.clone();
                                    drop(server);

                                    if let Err(_err) = tokio::time::timeout(
                                        Duration::from_secs(10),
                                        sessions.lock_owned(),
                                    )
                                    .await
                                    {
                                        warn!(
                                            "Deadlock detected on server_data::sessions : {}",
                                            id
                                        );
                                    }
                                    if let Err(_err) = tokio::time::timeout(
                                        Duration::from_secs(10),
                                        attach_write.lock_owned(),
                                    )
                                    .await
                                    {
                                        warn!(
                                            "Deadlock detected on server_data::attach_write : {}",
                                            id
                                        );
                                    }
                                    if let Err(_err) = tokio::time::timeout(
                                        Duration::from_secs(10),
                                        buffer.lock_owned(),
                                    )
                                    .await
                                    {
                                        warn!("Deadlock detected on server_data::buffer : {}", id);
                                    }
                                }
                                Err(_) => {
                                    warn!("Deadlock detected on server_data (this is normal if an image build is in progress): {}", id);
                                }
                            }
                        }
                    }
                    Err(_) => {
                        warn!("Deadlock detected on state::servers");
                    }
                }
            }
        });
    }

    #[cfg(feature = "ssl")]
    if config.ssl.enabled {
        axum_server::bind_rustls(
            config
                .bind
                .parse()
                .inspect_err(|_err| error!("Failed to parse socket address!"))
                .unwrap(),
            config
                .ssl
                .as_rustls()
                .await
                .inspect_err(|_err| error!("Failed to parse ssl certs!"))
                .unwrap(),
        )
        .serve(app.into_make_service())
        .await
        .inspect_err(|_err| error!("Failed to serve http server!"))
        .unwrap();
        return;
    }
    axum_server::bind(
        config
            .bind
            .parse()
            .inspect_err(|_err| error!("Failed to parse socket address!"))
            .unwrap(),
    )
    .serve(app.into_make_service())
    .await
    .inspect_err(|_err| error!("Failed to serve http server!"))
    .unwrap();
}

#[derive(Clone)]
pub(crate) struct DaemonState {
    pub(crate) config: Arc<DaemonConfig>,
    pub(crate) servers: Arc<Mutex<HashMap<u32, Arc<Mutex<ServerData>>>>>,
    pub(crate) storages: Arc<Mutex<HashMap<StorageIdentifier, StorageData>>>,
    pub(crate) docker: Arc<Docker>,
    pub(crate) conn: Arc<Pool<MySql>>,
    pub(crate) client: Arc<Client>,
}

#[derive(Clone)]
pub(crate) struct StorageData {
    pub(crate) lock: Arc<RwLock<()>>,
    pub(crate) value: Arc<(AtomicI64, AtomicI64)>,
    pub(crate) stats_lock: Arc<Mutex<()>>,
}

pub(crate) type ServerContainerWrite = Pin<Box<dyn AsyncWrite + Send>>;

pub(crate) struct ServerData {
    pub(crate) restart: bool,
    pub(crate) stopping: bool,
    pub(crate) size: (u32, u32),
    pub(crate) buffer: Arc<Mutex<AllocRingBuffer<u8>>>,
    pub(crate) attach_write: Arc<Mutex<Option<ServerContainerWrite>>>,
    pub(crate) sender: Arc<Sender<ChannelWriteData>>,
    pub(crate) stats_sender: Arc<Sender<ChannelWriteData>>,
    pub(crate) sessions: Arc<Mutex<HashMap<Uuid, TerminalSession>>>,
    pub(crate) stop_command: Option<String>,
    pub(crate) failure_counter: u8,
}

impl Default for ServerData {
    fn default() -> Self {
        Self {
            restart: false,
            stopping: false,
            attach_write: Arc::new(Mutex::new(None)),
            sender: Arc::new(Sender::new(400)),
            stats_sender: Arc::new(Sender::new(400)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            size: (170, 30),
            buffer: Arc::new(Mutex::new(AllocRingBuffer::new(
                crate::common::DAEMON_TERMINAL_BUFFER_LIMIT,
            ))),
            stop_command: None,
            failure_counter: 0,
        }
    }
}

// Some safeguards (macos and linux are supported)
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("This daemon only runs on Unix.");

#[cfg(all(not(debug_assertions), feature = "ssh_debug_auth_bypass"))]
compile_error!("SSH Auth Bypass was enabled in production!");

#[cfg(all(not(debug_assertions), feature = "user_debug_auth_bypass"))]
compile_error!("Websocket Auth Bypass was enabled in production!");
