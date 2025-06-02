pub use crate::storage::archive::compression::{ArchiveCompression, CompressionLevel};
use reqwest::Url;
use russh::keys::PrivateKey;
use russh::keys::ssh_key::private::{Ed25519Keypair, RsaKeypair};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub backend_url: Url,
    pub listeners: HashSet<SocketAddr>,
    pub origin: String,
    pub storage: HashMap<String, StorageEntry>,
    pub backup: HashMap<String, BackupEntry>,
    pub database: String,
    pub tls: Option<TlsConfig>,
    pub ssh: Option<SSHConfig>,
    #[serde(default)]
    pub network: NetworkConfig,
    pub mounts: Vec<MountPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountPoint {
    pub src: String,
    pub dest: String,
    pub mode: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SSHConfig {
    pub listeners: HashSet<SocketAddr>,
    pub host_key: Vec<SSHHostKey>,
    pub auth_cache_duration: u64,
    pub auth_url: reqwest::Url,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "method")]
pub enum StorageEntry {
    #[serde(rename = "directory")]
    Directory { path: String },
    #[cfg(feature = "btrfs")]
    #[serde(rename = "btrfs")]
    Btrfs {
        group: String,
        path: String,
        quota: bool,
    },
}

impl StorageEntry {
    pub fn path(&self) -> &String {
        match self {
            StorageEntry::Directory { path } => path,
            #[cfg(feature = "btrfs")]
            StorageEntry::Btrfs { path, .. } => path,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "method")]
pub enum BackupEntry {
    #[serde(rename = "archive")]
    Archive {
        compression: ArchiveCompression,
        path: String,
        order: Option<i8>,
    },
    #[cfg(feature = "btrfs")]
    #[serde(rename = "btrfs")]
    Btrfs {
        jit_compression: ArchiveCompression,
        path: String,
        group: String,
        quota: bool,
        order: Option<i8>,
    },
}

impl BackupEntry {
    pub fn path(&self) -> &String {
        match self {
            BackupEntry::Archive { path, .. } => path,
            #[cfg(feature = "btrfs")]
            BackupEntry::Btrfs { path, .. } => path,
        }
    }

    pub fn order(&self) -> Option<i8> {
        match self {
            BackupEntry::Archive { order, .. } => *order,
            #[cfg(feature = "btrfs")]
            BackupEntry::Btrfs { order, .. } => *order,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    pub private_key: PathBuf,
    pub certificate: PathBuf,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct NetworkConfig {
    // TODO: Assign dedicated ipv6 address to container, see #312
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SSHHostKey {
    #[serde(rename = "ed25519")]
    ED25519 { path: String },
    #[serde(rename = "rsa")]
    #[allow(clippy::upper_case_acronyms)]
    RSA { path: String },
}

impl SSHHostKey {
    pub async fn as_russh_key(&self) -> Result<PrivateKey, anyhow::Error> {
        use ed25519_dalek::ed25519::signature::rand_core::OsRng;
        use pkcs8::LineEnding;
        use russh::keys::ssh_encoding::{DecodePem, EncodePem};
        use std::io::ErrorKind;
        use tokio::fs::{File, OpenOptions};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut buffer = Vec::new();
        Ok(match self {
            SSHHostKey::ED25519 { path } => match File::open(path).await {
                Ok(mut file) => {
                    file.read_to_end(&mut buffer).await?;
                    PrivateKey::decode_pem(&buffer)?
                }
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    let keypair = Ed25519Keypair::random(&mut OsRng);
                    let mut file = OpenOptions::new()
                        .create(true)
                        .write(true)
                        .truncate(true)
                        .open(path)
                        .await?;
                    let priv_key: PrivateKey = keypair.into();
                    buffer.extend(priv_key.encode_pem_string(LineEnding::LF)?.into_bytes());
                    file.write_all(&buffer).await?;
                    priv_key
                }
                Err(err) => {
                    return Err(err.into());
                }
            },
            SSHHostKey::RSA { path } => match File::open(path).await {
                Ok(mut file) => {
                    file.read_to_end(&mut buffer).await?;
                    PrivateKey::decode_pem(&buffer)?
                }
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    let keypair = RsaKeypair::random(&mut OsRng, 4096)?;
                    let mut file = OpenOptions::new()
                        .create(true)
                        .write(true)
                        .truncate(true)
                        .open(path)
                        .await?;
                    let priv_key: PrivateKey = keypair.into();
                    buffer.extend(priv_key.encode_pem_string(LineEnding::LF)?.into_bytes());
                    file.write_all(&buffer).await?;
                    priv_key
                }
                Err(err) => return Err(err.into()),
            },
        })
    }
}
