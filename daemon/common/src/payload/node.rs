use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeRegistrationRequest {
    pub identifier: String,
    pub base_url: String,
    pub token: String,

    pub ssh_addr: String,
    pub ssh_port: u16,

    pub default_bind: String,
    pub default_host: String,

    pub storages: HashMap<String, ServerStorage>,
    #[cfg(feature = "backups")]
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

#[cfg(feature = "backups")]
#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
pub enum BackupStorage {
    #[cfg(feature = "btrfs")]
    #[serde(rename = "btrfs")]
    Btrfs(String),
    #[serde(rename = "archive")]
    Archive,
}

#[cfg(feature = "backups")]
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
