// SPDX-License-Identifier: AGPL-3.0-only
use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::{Host, Url};
use utoipa::{PartialSchema as _, ToSchema};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HostAndPort {
    #[schema(schema_with = String::schema)]
    pub host: Host,
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct NodeRegistrationRequest {
    pub cert: Vec<u8>,
    pub csr: Vec<u8>,

    pub default_bind: String,
    pub default_host: String,

    pub config: SetupNodeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SetupNodeConfig {
    /// The publicly reachable base URL of the daemon.
    pub base_url: Url,
    /// The publicly reachable SSH address, or `None` if SSH is disabled.
    pub ssh: Option<HostAndPort>,

    pub storages: HashMap<String, ServerStorage>,
    pub backups: HashMap<String, BackupStorage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BackendNodeConfig {
    /// The publicly reachable base URL of the daemon.
    pub base_url: Url,
    /// The publicly reachable SSH address, or `None` if SSH is disabled.
    pub ssh: Option<HostAndPort>,
    
    pub storages: Vec<String>,
    pub backups: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq, ToSchema)]
pub enum ServerStorage {
    #[serde(rename = "btrfs")]
    Btrfs(String),
    #[serde(rename = "directory")]
    Directory,
}

#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq, ToSchema)]
pub enum BackupStorage {
    #[serde(rename = "btrfs")]
    Btrfs(String),
    #[serde(rename = "archive")]
    Archive,
}

impl BackupStorage {
    #[must_use]
    pub fn is_compatible_with(&self, storage: &ServerStorage) -> bool {
        match (self, storage) {
            (Self::Btrfs(backup_group), ServerStorage::Btrfs(server_group)) => {
                server_group.eq(backup_group)
            }
            (Self::Btrfs(_), ServerStorage::Directory) => false,
            (Self::Archive, _) => true,
        }
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct BackendAuthServerPermissions: i32 {
        const TERMINAL_READ = 1 << 0;
        const TERMINAL_WRITE = 1 << 1;

        const FILES_READ = 1 << 2;
        const FILES_WRITE = 1 << 3;

        const BACKUP_DOWNLOAD = 1 << 8;
    }
}
