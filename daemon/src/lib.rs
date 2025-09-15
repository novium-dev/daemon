/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
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
pub mod key;
mod network;
mod schedule;
mod service;
pub mod sync;
mod terminal;
mod utils;
mod websocket;

use crate::sync::Mutex;
use crate::sync::RwLock;
use crate::sync::broadcast::Sender;
use bollard::Docker;
use core::pin::Pin;
use core::sync::atomic::AtomicI64;
use ed25519_dalek::SigningKey;
use reqwest::Client;
use ringbuffer::AllocRingBuffer;
use sqlx::{MySql, Pool};
use std::collections::HashMap;
use tokio::io::AsyncWrite;
use tokio::task::{AbortHandle, JoinSet};
use url::Url;

use crate::config::DaemonConfig;
use crate::storage::StorageIdentifier;
use crate::storage::sync::StorageTokens;
use crate::utils::ChannelWriteData;
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
    pub(crate) lock: Arc<RwLock<()>>,
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
    pub(crate) restart: bool,
    pub(crate) stopping: bool,
    #[cfg(not(feature = "no_tty_size_please"))]
    pub(crate) size: (u16, u16),
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
