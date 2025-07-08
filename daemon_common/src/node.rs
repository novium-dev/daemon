use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::{Host, Url};

#[derive(Debug, Serialize, Deserialize)]
pub struct HostAndPort {
    pub host: Host,
    pub port: u16,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(
    not(feature = "btrfs"),
    expect(
        clippy::zero_sized_map_values,
        reason = "single value enum if btrfs is disabled"
    )
)]
pub struct NodeRegistrationRequest {
    pub base_url: Url,

    pub ssh: Option<HostAndPort>,
    pub cert: Vec<u8>,
    pub csr: Vec<u8>,

    pub default_bind: String,
    pub default_host: String,

    pub storages: HashMap<String, ServerStorage>,
    pub backups: HashMap<String, BackupStorage>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(
    not(feature = "btrfs"),
    expect(
        clippy::zero_sized_map_values,
        reason = "single value enum if btrfs is disabled"
    )
)]
pub struct NodeUpdateRequest {
    pub base_url: Url,

    pub ssh: Option<HostAndPort>,

    pub storages: HashMap<String, ServerStorage>,
    pub backups: HashMap<String, BackupStorage>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
pub enum ServerStorage {
    #[cfg(feature = "btrfs")]
    #[serde(rename = "btrfs")]
    Btrfs(String),
    #[serde(rename = "directory")]
    Directory,
}

#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
pub enum BackupStorage {
    #[cfg(feature = "btrfs")]
    #[serde(rename = "btrfs")]
    Btrfs(String),
    #[serde(rename = "archive")]
    Archive,
}

impl BackupStorage {
    #[must_use]
    #[cfg_attr(
        not(feature = "btrfs"),
        expect(
            clippy::missing_const_for_fn,
            reason = "the eq call is only present with btrfs"
        )
    )]
    pub fn is_compatible_with(&self, storage: &ServerStorage) -> bool {
        match (self, storage) {
            #[cfg(feature = "btrfs")]
            (Self::Btrfs(backup_group), ServerStorage::Btrfs(server_group)) => {
                server_group.eq(backup_group)
            }
            #[cfg(feature = "btrfs")]
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

        const CONTROL_VIEW = 1 << 6;
        const CONTROL_POWER = 1 << 7;

        const BACKUP_DOWNLOAD = 1 << 8;
    }
}
