#![cfg_attr(not(feature = "trace_sync"), allow(clippy::disallowed_methods))]

pub mod config;
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
mod sync;
mod terminal;
mod utils;
mod websocket;

use crate::sync::Mutex;
use crate::sync::RwLock;
use crate::sync::broadcast::Sender;
use bollard::Docker;
use ed25519_dalek::SigningKey;
use reqwest::Client;
use ringbuffer::AllocRingBuffer;
use sqlx::{MySql, Pool};
use std::collections::HashMap;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::atomic::AtomicI64;
use tokio::io::AsyncWrite;
use tokio::task::{AbortHandle, JoinSet};
use url::Url;

use crate::config::DaemonConfig;
use crate::storage::StorageIdentifier;
use crate::utils::ChannelWriteData;
pub use daemon_common as common;
use daemon_common::FrameworkClientImpl;
use daemon_common::service::{DaemonServices, DaemonServicesURL, DaemonServicesURLWithUserKey};
use daemon_common::user::UserKeyProvider;
use std::sync::Arc;

pub const GIT_HASH: &str = env!("GIT_HASH");
#[derive(Clone)]
pub struct Daemon {
    pub(crate) config: Arc<DaemonConfig>,
    pub(crate) servers: Arc<Mutex<HashMap<u32, Arc<Mutex<ServerData>>>>>,
    pub(crate) storages: Arc<Mutex<HashMap<StorageIdentifier, StorageData>>>,
    pub(crate) server_task_cancellable: Arc<Mutex<JoinSet<()>>>,
    pub(crate) server_task: Arc<Mutex<JoinSet<()>>>,
    pub(crate) shutdown_task: Arc<Mutex<JoinSet<()>>>,
    pub(crate) shutdown_channel: Arc<Sender<()>>,
    pub(crate) schedule_task: Arc<Mutex<JoinSet<()>>>,
    pub(crate) schedules: Arc<Mutex<HashMap<u32, Arc<Mutex<ServerScheduleData>>>>>,
    pub(crate) docker: Arc<Docker>,
    pub(crate) conn: Arc<Pool<MySql>>,
    pub(crate) client: Arc<Client>,
    pub(crate) base_url: Url,
    pub(crate) user_key: SigningKey,
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
    #[cfg(not(feature = "no_tty_size_please"))]
    pub(crate) size: (u32, u32),
    pub(crate) buffer: Arc<Mutex<AllocRingBuffer<u8>>>,
    pub(crate) attach_write: Arc<Mutex<Option<ServerContainerWrite>>>,
    pub(crate) sender: Arc<Sender<ChannelWriteData>>,
    pub(crate) stats_sender: Arc<Sender<ChannelWriteData>>,
    #[cfg(not(feature = "no_tty_size_please"))]
    pub(crate) sessions: Arc<Mutex<HashMap<uuid::Uuid, terminal::TerminalSession>>>,
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
            #[cfg(not(feature = "no_tty_size_please"))]
            sessions: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(not(feature = "no_tty_size_please"))]
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

#[doc(hidden)]
pub mod internal {
    pub use crate::service::middleware::middleware;
}
