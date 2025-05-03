#![warn(clippy::string_to_string)]
#![cfg_attr(feature = "trace_sync", deny(clippy::disallowed_methods))]
#![cfg_attr(not(feature = "trace_sync"), allow(clippy::disallowed_methods))]

use crate::sync::Mutex;
use crate::sync::RwLock;
use crate::sync::broadcast::Sender;
use bollard::Docker;
use clap::{Arg, ArgAction, ArgMatches, Command, command};
use ed25519_dalek::SigningKey;
use reqwest::Client;
use ringbuffer::AllocRingBuffer;
use russh::server::{Config, Server as SSHServer};
use russh::{MethodKind, MethodSet, SshId};
use sqlx::{MySql, Pool};
use std::collections::HashMap;
use std::env;
use std::ops::Deref;
use std::pin::Pin;
use std::process::exit;
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWrite};
use tokio::task::{AbortHandle, JoinSet};
use tracing::{error, info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use url::Url;
use uuid::Uuid;

use crate::config::DaemonConfig;
use crate::storage::StorageIdentifier;
use crate::terminal::TerminalSession;
use crate::utils::ChannelWriteData;
pub use daemon_common as common;
use daemon_common::FrameworkClientImpl;
use daemon_common::service::{DaemonServices, DaemonServicesURL, DaemonServicesURLWithUserKey};
use daemon_common::user::UserKeyProvider;

mod config;
mod database;
mod server;

mod files;
mod sql;
mod ssh;
mod storage;

mod ctlsock;
pub mod init;
mod network;
mod schedule;
mod service;
pub(crate) mod setup;
mod sync;
mod terminal;
mod utils;
mod web;
mod websocket;

const GIT_HASH: &str = env!("GIT_HASH");

fn main() {
    let matches = command!()
        .about("hosting-system daemon")
        .args_override_self(true)
        .subcommand(
            Command::new("setup")
                .args_override_self(true)
                .short_flag('s')
                .long_flag("setup")
                .about("Start the setup wizard")
                .arg(
                    Arg::new("backend")
                        .short('b')
                        .long("backend")
                        .env("BACKEND_URL")
                        .help("The public accessible backend url")
                        .action(ArgAction::Set)
                        .required(false),
                )
                .arg(
                    Arg::new("code")
                        .short('c')
                        .long("code")
                        .env("REGISTRATION_CODE")
                        .help("The registration code")
                        .action(ArgAction::Set)
                        .required(false),
                )
                .arg(
                    Arg::new("skip-mariadb-plugin")
                        .short(None)
                        .long("skip-mariadb-plugin")
                        .env("SKIP_MARIADB_PLUGIN")
                        .help("Skips the mariadb plugin install step")
                        .action(ArgAction::SetTrue)
                        .required(false),
                ),
        )
        .arg(
            Arg::new("debug")
                .short('d')
                .long("debug")
                .env("DEBUG")
                .help("Enables debug output")
                .required(false)
                .action(ArgAction::SetTrue)
                .default_value(if cfg!(debug_assertions) {
                    "true"
                } else {
                    "false"
                }),
        )
        .arg(
            Arg::new("skip-capability-setup")
                .short(None)
                .long("skip-capability-setup")
                .env("SKIP_CAPABILITY_SETUP")
                .help("Skips the setuid")
                .required(false)
                .action(ArgAction::SetTrue),
        )
        .get_matches();

    if !matches.get_flag("skip-capability-setup") {
        cap_setup();
    }

    let reg = tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::from_level(
            if matches.get_flag("debug") {
                tracing::Level::DEBUG
            } else {
                tracing::Level::INFO
            },
        ))
        .with(tracing_subscriber::fmt::layer());

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Unable to set ring as default crypto provider");

    #[cfg(not(all(feature = "sentry", not(debug_assertions))))]
    reg.init();

    #[cfg(all(feature = "sentry", not(debug_assertions)))]
    let guard = {
        reg.with(sentry::integrations::tracing::layer()).init();
        let sentry = sentry::init((
            "https://79a1f560730aef34fdc4e4a3f3d9ce92@o4507044844404736.ingest.us.sentry.io/4508313795297280",
            sentry::ClientOptions {
                release: sentry::release_name!(),
                ..Default::default()
            },
        ));
        if sentry.is_enabled() {
            info!("Enabled Sentry integration");
        }
        sentry
    };

    main0(matches);

    // Ensure it gets dropped, only after the daemon exists
    #[cfg(all(feature = "sentry", not(debug_assertions)))]
    drop(guard);
}

#[tokio::main]
async fn main0(matches: ArgMatches) {
    match matches.subcommand() {
        Some(("setup", _)) => {
            tokio::task::spawn_blocking(move || {
                let setup_matches = matches.subcommand_matches("setup").unwrap();
                let backend = setup_matches.get_one("backend");
                let code = setup_matches.get_one("code");
                let skip_mariadb_plugin = setup_matches.get_flag("skip-mariadb-plugin");
                setup::setup(backend.cloned(), code.cloned(), skip_mariadb_plugin);
            })
            .await
            .expect("Setup wizard failed");
        }
        None => {
            run().await;
        }
        _ => unreachable!(),
    }
}

async fn run() {
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

    let daemon = Daemon::init(config).await.unwrap();

    info!("Starting Webserver...");

    daemon.serve_http().await.unwrap();

    if let Some(ssh) = daemon.config.ssh.clone() {
        let daemon = daemon.clone();
        tokio::task::spawn(async move {
            let mut keys = Vec::with_capacity(ssh.host_key.len());
            for key in &ssh.host_key {
                keys.push(match key.as_russh_key().await {
                    Ok(key) => key,
                    Err(err) => {
                        error!("Failed to start ssh server (key): {:?}", err);
                        exit(-1);
                    }
                });
                tracing::debug!("Loaded ssh key: {:?}", key);
            }
            let mut methods = MethodSet::empty();
            methods.push(MethodKind::Password);
            let ssh_config = Arc::new(Config {
                keys,
                server_id: SshId::Standard(format!("{}{}", common::SSH_BANNER_PREFIX, GIT_HASH)),
                methods,
                ..Config::default()
            });
            for listener in ssh.listeners {
                if let Err(err) = (crate::ssh::server::DaemonSSHServer {
                    cache_invalidation: Duration::from_millis(ssh.auth_cache_duration),
                    state: daemon.clone(),
                })
                .run_on_address(ssh_config.clone(), &listener)
                .await
                {
                    error!("Failed to start ssh server (bind): {:?}", err);
                    exit(-1);
                }
            }
        });
    }

    daemon.hook_signals();

    daemon.await_shutdown().await;
}

#[derive(Clone)]
pub(crate) struct Daemon {
    pub(crate) config: Arc<DaemonConfig>,
    pub(crate) servers: Arc<Mutex<HashMap<u32, Arc<Mutex<ServerData>>>>>,
    pub(crate) storages: Arc<Mutex<HashMap<StorageIdentifier, StorageData>>>,
    pub(crate) server_task: Arc<Mutex<JoinSet<()>>>,
    pub(crate) shutdown_task: Arc<Mutex<JoinSet<()>>>,
    pub(crate) shutdown_channel: Arc<Sender<()>>,
    pub(crate) schedule_task: Arc<Mutex<JoinSet<()>>>,
    pub(crate) schedules: Arc<Mutex<HashMap<u32, Arc<Mutex<ServerScheduleData>>>>>,
    pub(crate) docker: Arc<Docker>,
    pub(crate) conn: Arc<Pool<MySql>>,
    pub(crate) client: Arc<Client>,
    base_url: Url,
    user_key: SigningKey,
}

impl DaemonServices for Daemon {}
impl DaemonServicesURL for Daemon {}
impl DaemonServicesURLWithUserKey for Daemon {}
impl FrameworkClientImpl for Daemon {
    fn client(&self) -> &Client {
        self.client.deref()
    }

    fn base_url(&self) -> &Url {
        &self.base_url
    }
}
impl UserKeyProvider for Daemon {
    fn node_id(&self) -> &str {
        &self.config.identifier
    }

    fn key(&self) -> &SigningKey {
        &self.user_key
    }
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

#[derive(Default)]
pub(crate) struct ServerScheduleData {
    pub(crate) schedules: HashMap<u32, AbortHandle>,
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

// Some safeguards
// Linux Maintainer: Markus Probst <markustieger@gmail.com>
// Macos Maintainer: InvalidJoker <0limitdev@gmail.com>
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("This daemon only runs on Unix.");

#[cfg(all(not(debug_assertions), feature = "ssh_debug_auth_bypass"))]
compile_error!("SSH Auth Bypass was enabled in production!");

fn cap_setup() {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        if std::fs::metadata("/proc/self/").unwrap().uid() != 0 {
            return;
        }
        capctl::prctl::set_keepcaps(true).expect("Unable to keep capabilities after setuid");
        let mut capstate =
            capctl::CapState::get_current().expect("Unable to retrieve current capabilities");
        capstate.permitted.add(capctl::Cap::NET_BIND_SERVICE);
        capstate.permitted.add(capctl::Cap::DAC_OVERRIDE);
        capstate.permitted.add(capctl::Cap::FOWNER);
        capstate.permitted.add(capctl::Cap::CHOWN);
        capstate.permitted.add(capctl::Cap::SYS_CHROOT);
        capstate.permitted.add(capctl::Cap::SYS_ADMIN);
        capstate
            .set_current()
            .expect("Unable to set permitted capabilities");

        capctl::cap_set_ids(
            Some(crate::common::CONTAINER_DEFAULT_UID as u32),
            Some(crate::common::CONTAINER_DEFAULT_GID as u32),
            None,
        )
        .expect("Unable to setuid current process");

        let mut capstate = capctl::CapState::get_current().unwrap();
        capstate.effective.add(capctl::Cap::NET_BIND_SERVICE);
        capstate.effective.add(capctl::Cap::DAC_OVERRIDE);
        capstate.effective.add(capctl::Cap::FOWNER);
        capstate.effective.add(capctl::Cap::CHOWN);
        capstate.effective.add(capctl::Cap::SYS_CHROOT);
        capstate.effective.add(capctl::Cap::SYS_ADMIN);
        capstate
            .set_current()
            .expect("Unable to set effective capabilities");
    }
}
