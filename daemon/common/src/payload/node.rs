use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use url::{Host, Url};

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeRegistrationRequest {
    pub identifier: String,
    pub base_url: Url,
    pub token: String,

    pub ssh: Option<(Host, u16)>,

    pub default_bind: String,
    pub default_host: String,

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
    pub fn is_compatible_with(&self, storage: &ServerStorage) -> bool {
        match (self, storage) {
            #[cfg(feature = "btrfs")]
            (BackupStorage::Btrfs(backup_group), ServerStorage::Btrfs(server_group)) => {
                server_group.eq(backup_group)
            }
            #[cfg(feature = "btrfs")]
            (BackupStorage::Btrfs(_), ServerStorage::Directory) => false,
            (BackupStorage::Archive, _) => true,
        }
    }
}
