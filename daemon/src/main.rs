mod config;
mod database;
mod dirfd;
mod schema;
mod server;
mod ssh;
mod storage;
mod terminal;
mod utils;
mod web;

use crate::config::DaemonConfig;
use crate::schema::FetchedServer;
use crate::ssh::DaemonSSHServer;
use crate::terminal::{ChannelWriteData, TerminalSession};
use axum::http::{HeaderValue, Method};
use axum::routing::{delete, get, patch, put};
use axum::Router;
use bollard::Docker;
use diesel::connection::SimpleConnection;
use diesel::{Connection, MysqlConnection};
use ed25519_dalek::pkcs8::{DecodePrivateKey, KeypairBytes};
use ed25519_dalek::SigningKey;
use env_logger::Env;
use libc::SIGINT;
use log::{info, warn};
use russh::server::{Config, Server as SSHServer};
use russh_keys::key::KeyPair;
use signal_hook::iterator::Signals;
use std::collections::HashMap;
use std::error::Error;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::process::exit;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWrite;
use tokio::runtime::Handle;
use tokio::sync::broadcast::Sender;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tower_http::cors::{AllowHeaders, AllowOrigin, CorsLayer};
use tower_http::validate_request::ValidateRequestHeaderLayer;
use uuid::Uuid;

include!("../../config.rs");

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let env = Env::default().filter_or("LOG_LEVEL", "info");
    env_logger::init_from_env(env);

    let secret_key = KeypairBytes::from_pkcs8_pem(include_str!("test_ed25519.pem")).unwrap();
    let secret_key = secret_key.secret_key;
    let test_key = SigningKey::from_bytes(&secret_key);

    let config = DaemonConfig {
        identifier: String::from("test-node"),
        token: String::from("test"),
        websocket_key: test_key.verifying_key(),
        websocket_allow_duration: Duration::from_secs(60 * 60 * 24 * 365 * 12).as_millis(), // make like 60 seconds, THIS IS ONLY FOR TESTING.
        origin: String::from("http://localhost:3000"),
        storage_path: String::from("./storage"),
        btrfs: false,
        bind: String::from("0.0.0.0:8080"),
        ssh_bind: String::from("0.0.0.0:2022"),
        database: String::from("mysql://localhost/daemon?unix_socket=mysql/sockets/mysqld.sock"),
    };

    info!("Starting Daemon...");

    let servers = HashMap::new();

    info!("Connecting to Docker...");
    let docker = bollard::Docker::connect_with_local_defaults()?;

    info!("Establishing Database Connection...");
    let conn = MysqlConnection::establish(&config.database)?;

    info!("Starting Webserver...");

    let config = Arc::new(config);
    let state = DaemonState {
        config: config.clone(),
        servers: Arc::new(Mutex::new(servers)),
        docker: Arc::new(docker),
        conn: Arc::new(Mutex::new(conn)),
    };

    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::exact(HeaderValue::from_str(&config.origin)?))
        .allow_methods([Method::GET])
        .allow_headers(AllowHeaders::any());

    let app = Router::new()
        .nest(
            "/server",
            Router::new()
                .route("/", put(web::create_server))
                .route("/:server_id", get(web::server_info))
                .route("/:server_id", patch(web::modify_server))
                .route("/:server_id", delete(web::delete_server))
                .route("/:server_id/start", get(web::start_server))
                .route("/:server_id/stop", get(web::stop_server))
                .route("/:server_id/kill", get(web::kill_server))
                .route("/:server_id/restart", get(web::restart_server))
                .route_layer(ValidateRequestHeaderLayer::bearer(&config.token)),
        )
        .nest(
            "/user",
            Router::new()
                .layer(cors)
                .route("/server/:server_id/ws", get(web::websocket::websocket)),
        )
        .with_state(state.clone());

    state.conn.lock().await.batch_execute(
        &include_str!("init.sql").replace("{{MARIADB_PLUGIN_NAME}}", crate::MARIADB_PLUGIN_NAME),
    )?;
    let state = Arc::new(state);

    let state1 = state.clone();
    let config1 = config.clone();
    tokio::task::spawn(async move {
        DaemonSSHServer { state: state1 }
            .run_on_address(
                Arc::new(Config {
                    keys: vec![KeyPair::Ed25519(test_key)],
                    ..Config::default()
                }),
                &config1.ssh_bind,
            )
            .await
    });

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

    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

#[derive(Clone)]
pub(crate) struct DaemonState {
    pub(crate) config: Arc<DaemonConfig>,
    pub(crate) servers: Arc<Mutex<HashMap<u32, Arc<Mutex<ServerData>>>>>,
    pub(crate) docker: Arc<Docker>,
    pub(crate) conn: Arc<Mutex<MysqlConnection>>,
}

pub(crate) type ServerContainerWrite = Pin<Box<dyn AsyncWrite + Send>>;

pub(crate) struct ServerData {
    pub(crate) restart: bool,
    pub(crate) stopping: bool,
    pub(crate) size: (u32, u32),
    pub(crate) buffer: Arc<Mutex<Vec<u8>>>,
    pub(crate) attach_write: Arc<Mutex<Option<ServerContainerWrite>>>,
    pub(crate) sender: Arc<Sender<ChannelWriteData>>,
    pub(crate) sessions: Arc<Mutex<HashMap<Uuid, TerminalSession>>>,
}

impl Default for ServerData {
    fn default() -> Self {
        Self {
            restart: false,
            stopping: false,
            attach_write: Arc::new(Mutex::new(None)),
            sender: Arc::new(Sender::new(400)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
            size: (170, 30),
            buffer: Arc::new(Mutex::new(Vec::new())),
        }
    }
}
