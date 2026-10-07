// SPDX-License-Identifier: AGPL-3.0-only

pub mod config;
#[cfg(feature = "databases")]
mod database;
mod server;

mod files;
mod sql;
mod ssh;
mod storage;

mod cgroup;
mod ctlsock;
pub mod init;
mod integrity;
mod network;
mod schedule;
mod service;
pub mod sync;
mod terminal;
mod utils;
mod websocket;

use bollard::Docker;
use core::pin::Pin;
use core::sync::atomic::AtomicI64;
use core::sync::atomic::AtomicU16;
use core::sync::atomic::{AtomicBool, AtomicU8};
use daemon_common::service::files::task::StorageTask;
use daemon_common::service::server::task::ServerTask;
use ed25519_dalek::SigningKey;
use reqwest::Client;
use ringbuffer::AllocRingBuffer;
use sqlx::{MySql, Pool};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;
use tokio::io::AsyncWrite;
use tokio::sync::Mutex;
use tokio::sync::broadcast::Sender;
use tokio::task::{AbortHandle, JoinSet};
use url::Url;

use crate::config::DaemonConfig;
use crate::storage::StorageIdentifier;
use crate::storage::sync::StorageTokens;
use crate::sync::task::TaskRwLock;
use crate::utils::ChannelWriteData;
use crate::utils::LinuxOnly;
pub use daemon_common as common;
use daemon_common::key::{DaemonKey, DaemonRootKey};
use daemon_common::service::ZelusClientImpl;
use daemon_common::service::{DaemonServices, DaemonServicesURL, DaemonServicesURLWithUserKey};
use daemon_common::user::UserKeyProvider;
use std::sync::Arc;
use uuid::Uuid;

pub const GIT_HASH: &str = env!("GIT_HASH");
#[derive(Clone)]
pub struct Daemon<T = ()>
where
    T: DaemonExtra,
{
    pub(crate) config: Arc<DaemonConfig>,
    pub(crate) servers: Arc<Mutex<HashMap<u32, Arc<ServerData>>>>,
    pub(crate) storages: Arc<Mutex<HashMap<StorageIdentifier, StorageData>>>,
    pub(crate) server_task_cancellable: Arc<Mutex<JoinSet<()>>>,
    pub(crate) server_task: Arc<Mutex<JoinSet<()>>>,
    pub(crate) shutdown_task: Arc<Mutex<JoinSet<()>>>,
    pub(crate) shutdown_channel: Arc<Sender<()>>,
    pub(crate) schedule_task: Arc<Mutex<JoinSet<()>>>,
    pub(crate) schedules: Arc<Mutex<HashMap<u32, Arc<Mutex<ServerScheduleData>>>>>,
    pub(crate) docker: Arc<Docker>,
    pub(crate) dbus: LinuxOnly<zbus::Connection>,
    pub(crate) conn: Arc<Pool<MySql>>,
    pub(crate) runtime_dir: PathBuf,
    pub(crate) client: Arc<Client>,
    pub(crate) started: Instant,
    pub(crate) extra: Arc<T>,

    pub(crate) root: Arc<DaemonRootKey>,
    pub(crate) key: Arc<DaemonKey>,
}

#[derive(Clone)]
pub struct WithUserKey {
    pub(crate) base_url: Url,
    pub(crate) user_key: SigningKey,
}

pub(crate) trait DaemonExtra: Clone + Send + Sync + 'static {}

impl DaemonExtra for () {}
impl DaemonExtra for WithUserKey {}

impl<T: DaemonExtra> DaemonServices for Daemon<T> {
    fn id(&self) -> Uuid {
        self.key.id
    }
}
impl DaemonServicesURL for Daemon<WithUserKey> {}
impl DaemonServicesURLWithUserKey for Daemon<WithUserKey> {}
impl ZelusClientImpl for Daemon<WithUserKey> {
    fn client(&self) -> &Client {
        &self.client
    }

    fn base_url(&self) -> &Url {
        &self.extra.base_url
    }
}
impl UserKeyProvider for Daemon<WithUserKey> {
    fn node_id(&self) -> &Uuid {
        &self.key.id
    }

    fn key(&self) -> &SigningKey {
        &self.extra.user_key
    }
}

#[derive(Clone)]
pub(crate) struct StorageData {
    pub(crate) lock: Arc<TaskRwLock<StorageTask, ()>>,
    pub(crate) value: Arc<(AtomicI64, AtomicI64)>,
    pub(crate) stats_lock: Arc<Mutex<()>>,
    pub(crate) notify: Sender<StorageNotification>,
    pub(crate) tokens: Arc<StorageTokens>,
}

#[derive(Debug, Clone)]
#[expect(dead_code, reason = "maybe useful later")]
pub(crate) enum StorageNotification {
    Create {
        parent_inode: u64,
        inode: u64,
    },
    Rename {
        inode: u64,
        from_parent_inode: u64,
        to_parent_inode: u64,
    },
    RenameFrom {
        parent_inode: Option<u64>,
        inode: Option<u64>,
    },
    Update {
        inode: Option<u64>,
        parent_inode: Option<u64>,
    },
    Delete {
        dir: bool,
        inode: u64,
        parent_inode: u64,
    },
}

pub(crate) type ServerContainerWrite = Pin<Box<dyn AsyncWrite + Send>>;

pub(crate) struct ServerData {
    pub(crate) lock: Arc<TaskRwLock<ServerTask, ()>>,
    pub(crate) restart: AtomicBool,
    pub(crate) stopping: AtomicBool,
    pub(crate) size: (AtomicU16, AtomicU16), // TODO: RwLock, possible race condition + store after completion
    pub(crate) buffer: Arc<Mutex<AllocRingBuffer<u8>>>,
    pub(crate) attach_write: Arc<Mutex<Option<ServerContainerWrite>>>,
    pub(crate) sender: Arc<Sender<ChannelWriteData>>,
    pub(crate) stats_sender: Arc<Sender<ChannelWriteData>>,
    pub(crate) sessions: Arc<Mutex<HashMap<uuid::Uuid, terminal::TerminalSession>>>,
    pub(crate) stop_command: Mutex<Option<String>>,
    pub(crate) failure_counter: AtomicU8,
}

#[derive(Default)]
pub(crate) struct ServerScheduleData {
    pub(crate) schedules: HashMap<u32, AbortHandle>,
}

impl Default for ServerData {
    fn default() -> Self {
        Self {
            lock: Arc::default(),
            restart: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            attach_write: Arc::new(Mutex::new(None)),
            sender: Arc::new(Sender::new(400)),
            stats_sender: Arc::new(Sender::new(400)),
            sessions: Arc::new(Mutex::new(HashMap::new())),

            size: (AtomicU16::new(170), AtomicU16::new(30)),
            buffer: Arc::new(Mutex::new(AllocRingBuffer::new(
                crate::common::DAEMON_TERMINAL_BUFFER_LIMIT,
            ))),
            stop_command: Mutex::default(),
            failure_counter: AtomicU8::new(0),
        }
    }
}

// Some safeguards
// Linux Maintainer: Markus Probst <markus.probst@posteo.de>
// Macos Maintainer: InvalidJoker <0limitdev@gmail.com>
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("This daemon only runs on Unix.");

#[cfg(all(not(debug_assertions), feature = "ssh_debug_auth_bypass"))]
compile_error!("SSH Auth Bypass was enabled in production!");

#[doc(hidden)]
pub mod internal {
    pub use crate::service::middleware::middleware;
}
