#![forbid(unsafe_code)]

use axum::http::{HeaderValue, Method};
use bollard::Docker;
use diesel_async::{AsyncConnection, AsyncMysqlConnection};
use env_logger::Env;
use libc::SIGINT;
use log::{info, warn};
#[cfg(feature = "ssh")]
use reqwest::Client;
#[cfg(feature = "ssh")]
use russh::server::{Config, Server as SSHServer};
use signal_hook::iterator::Signals;
use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::process::exit;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWrite};
use tokio::runtime::Handle;
use tokio::sync::broadcast::Sender;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinSet;
use tower_http::cors::{AllowHeaders, AllowOrigin, CorsLayer};
use tower_http::validate_request::ValidateRequestHeaderLayer;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_swagger_ui::SwaggerUi;
use uuid::Uuid;

pub use daemon_common as common;

use crate::config::DaemonConfig;
use crate::schema::FetchedServer;
#[cfg(feature = "ssh")]
use crate::ssh::DaemonSSHServer;
use crate::storage::StorageIdentifier;
use crate::terminal::TerminalSession;
use crate::utils::ChannelWriteData;

mod config;
#[cfg(feature = "database")]
mod database;
mod schema;
mod server;

#[cfg(feature = "files")]
mod files;
mod sql;
#[cfg(feature = "ssh")]
mod ssh;
mod storage;

mod terminal;
mod utils;
mod web;

#[derive(OpenApi)]
#[openapi()]
struct ApiDoc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    #[cfg(not(debug_assertions))]
    let env = Env::default().filter_or("LOG_LEVEL", "info");
    #[cfg(debug_assertions)]
    let env = Env::default().filter_or("LOG_LEVEL", "debug");
    env_logger::init_from_env(env);

    let mut config_buffer = String::new();
    File::open("config.toml")
        .await?
        .read_to_string(&mut config_buffer)
        .await
        .unwrap();
    let config: DaemonConfig = toml::from_str(&config_buffer)?;

    info!("Starting Daemon...");

    let servers = HashMap::new();

    info!("Connecting to Docker...");
    let docker = bollard::Docker::connect_with_local_defaults()?;

    info!("Establishing Database Connection...");
    let conn = AsyncMysqlConnection::establish(&config.database).await?;

    info!("Starting Webserver...");

    let config = Arc::new(config);
    let state = DaemonState {
        config: config.clone(),
        servers: Arc::new(Mutex::new(servers)),
        storages: Arc::new(Mutex::new(HashMap::new())),
        docker: Arc::new(docker),
        conn: Arc::new(Mutex::new(conn)),
        #[cfg(feature = "ssh")]
        client: Client::default(),
    };

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::exact(HeaderValue::from_str(&config.origin)?))
        .allow_methods([Method::GET])
        .allow_headers(AllowHeaders::any());

    let (mut router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(web::config())
        .route_layer(ValidateRequestHeaderLayer::bearer(&config.token))
        .merge(web::user_config().layer(cors))
        .split_for_parts();

    #[cfg(debug_assertions)]
    {
        router = router.merge(SwaggerUi::new("/docs").url("/docs/openapi.json", api));
    }

    let app = router.with_state(state.clone());

    sql::init(state.conn.lock().await.deref_mut()).await?;
    let state = Arc::new(state);

    #[cfg(feature = "ssh")]
    {
        let state1 = state.clone();
        let config1 = config.clone();
        tokio::task::spawn(async move {
            let mut keys = Vec::with_capacity(config1.ssh.host_key.len());
            for key in &config1.ssh.host_key {
                keys.push(key.as_russh_key().await?);
            }
            DaemonSSHServer {
                state: state1,
                cache_invalidation: Duration::from_millis(config1.ssh.auth_cache_duration),
            }
            .run_on_address(
                Arc::new(Config {
                    keys,
                    ..Config::default()
                }),
                &config1.ssh.bind,
            )
            .await
        });
    }

    let state1 = state.clone();
    let mut signal = Signals::new([SIGINT]).unwrap();
    tokio::task::spawn_blocking(move || {
        if signal.forever().next().is_some() {
            info!("Interrupt received. Shutting down all servers...");
            Handle::current().block_on(async move {
                let servers = state1.servers.lock().await;
                let mut set = JoinSet::new();
                for (id, server) in servers.deref() {
                    let mut server = server.clone().lock_owned().await;
                    let docker = state1.docker.clone();
                    let id = *id;
                    set.spawn(async move {
                        let _ = FetchedServer {
                            id,
                            ..FetchedServer::default()
                        }
                        .stop(&docker, server.deref_mut())
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
    if cfg!(debug_assertions) {
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
                                    warn!("Deadlock detected on server_data: {}", id);
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
        axum_server::bind_rustls(config.bind.parse()?, config.ssl.as_rustls().await?)
            .serve(app.into_make_service())
            .await?;
        return Ok(());
    }
    axum_server::bind(config.bind.parse()?)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

#[derive(Clone)]
pub(crate) struct DaemonState {
    pub(crate) config: Arc<DaemonConfig>,
    pub(crate) servers: Arc<Mutex<HashMap<u32, Arc<Mutex<ServerData>>>>>,
    pub(crate) storages: Arc<Mutex<HashMap<StorageIdentifier, Arc<RwLock<()>>>>>,
    pub(crate) docker: Arc<Docker>,
    pub(crate) conn: Arc<Mutex<AsyncMysqlConnection>>,
    #[cfg(feature = "ssh")]
    pub(crate) client: Client,
}

pub(crate) type ServerContainerWrite = Pin<Box<dyn AsyncWrite + Send>>;

pub(crate) struct ServerData {
    pub(crate) restart: bool,
    pub(crate) stopping: bool,
    pub(crate) size: (u32, u32),
    pub(crate) buffer: Arc<Mutex<VecDeque<u8>>>,
    pub(crate) attach_write: Arc<Mutex<Option<ServerContainerWrite>>>,
    pub(crate) sender: Arc<Sender<ChannelWriteData>>,
    pub(crate) stats_sender: Arc<Sender<ChannelWriteData>>,
    pub(crate) sessions: Arc<Mutex<HashMap<Uuid, TerminalSession>>>,
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
            buffer: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
}

// Some safeguards
#[cfg(not(target_os = "linux"))]
compile_error!("This daemon only runs on Linux.");

#[cfg(all(not(debug_assertions), feature = "ssh_debug_auth_bypass"))]
compile_error!("SSH Auth Bypass was enabled in production!");

#[cfg(all(not(debug_assertions), feature = "user_debug_auth_bypass"))]
compile_error!("Websocket Auth Bypass was enabled in production!");
