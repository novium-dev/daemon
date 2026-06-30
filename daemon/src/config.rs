// SPDX-License-Identifier: AGPL-3.0-only
pub use crate::storage::archive::compression::{ArchiveCompression, CompressionLevel};
use axum::http::HeaderValue;
use core::net::SocketAddr;
use daemon_common::node::{BackendNodeConfig, BackupStorage, HostAndPort, ServerStorage};
use reqwest::Url;
use russh::keys::PrivateKey;
use russh::keys::ssh_key::private::{Ed25519Keypair, RsaKeypair};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::sign::CertifiedKey;
use serde::{Deserialize, Serialize, de, ser};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub backend_url: Url,
    /// The publicly reachable base URL of this daemon (reported to the backend).
    pub public_url: Url,
    pub listeners: HashSet<SocketAddr>,
    /// Origins whitelisted for CORS requests against this daemon.
    #[serde(deserialize_with = "deserialize_headers")]
    #[serde(serialize_with = "serialize_headers")]
    pub origins: Vec<HeaderValue>,
    pub storage: HashMap<String, StorageEntry>,
    pub backup: HashMap<String, BackupEntry>,
    pub database: String,
    pub tls: Option<TlsConfig>,
    pub ssh: Option<SSHConfig>,
    #[serde(default)]
    pub network: NetworkConfig,
    pub mounts: Vec<MountPoint>,
}

impl DaemonConfig {
    /// Builds the logical [`NodeConfig`] reported to the backend during a backend-driven update.
    #[must_use]
    pub fn to_node_config(&self) -> BackendNodeConfig {
        BackendNodeConfig {
            base_url: self.public_url.clone(),
            ssh: self.ssh.as_ref().map(|ssh| ssh.public.clone()),
            storages: self.storage.keys().cloned().collect(),
            backups: self.backup.keys().cloned().collect(),
        }
    }
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
    /// The publicly reachable SSH address (reported to the backend).
    pub public: HostAndPort,
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
    #[must_use]
    pub const fn path(&self) -> &String {
        match self {
            Self::Directory { path } => path,
            #[cfg(feature = "btrfs")]
            Self::Btrfs { path, .. } => path,
        }
    }

    #[must_use]
    pub fn as_server_storage(&self) -> ServerStorage {
        match self {
            Self::Directory { .. } => ServerStorage::Directory,
            #[cfg(feature = "btrfs")]
            Self::Btrfs { group, .. } => ServerStorage::Btrfs(group.clone()),
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
    #[must_use]
    pub const fn path(&self) -> &String {
        match self {
            Self::Archive { path, .. } => path,
            #[cfg(feature = "btrfs")]
            Self::Btrfs { path, .. } => path,
        }
    }

    #[must_use]
    pub const fn order(&self) -> Option<i8> {
        match self {
            Self::Archive { order, .. } => *order,
            #[cfg(feature = "btrfs")]
            Self::Btrfs { order, .. } => *order,
        }
    }

    #[must_use]
    pub fn as_backup_storage(&self) -> BackupStorage {
        match self {
            Self::Archive { .. } => BackupStorage::Archive,
            #[cfg(feature = "btrfs")]
            Self::Btrfs { group, .. } => BackupStorage::Btrfs(group.clone()),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    pub private_key: PathBuf,
    pub certificate: PathBuf,
}

impl TlsConfig {
    pub async fn load_certs(&self) -> Result<CertifiedKey, io::Error> {
        let key = tokio::fs::read(&self.private_key).await?;
        let cert = tokio::fs::read(&self.certificate).await?;

        let key = PrivateKeyDer::from_pem_slice(&key).map_err(io::Error::other)?;
        let cert = CertificateDer::pem_slice_iter(&cert)
            .collect::<Result<Vec<_>, _>>()
            .map_err(io::Error::other)?;
        CertifiedKey::from_der(
            cert,
            key,
            rustls::crypto::CryptoProvider::get_default().expect("No default crypto provider set"),
        )
        .map_err(io::Error::other)
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
#[expect(clippy::empty_structs_with_brackets, reason = "not yet implemented")]
pub struct NetworkConfig {
    // TODO: Assign dedicated ipv6 address to container, see #312
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SSHHostKey {
    #[serde(rename = "ed25519")]
    Ed25519 { path: String },
    #[serde(rename = "rsa")]
    Rsa { path: String },
}

impl SSHHostKey {
    pub async fn as_russh_key(&self) -> Result<PrivateKey, anyhow::Error> {
        use pkcs8::LineEnding;
        use russh::keys::ssh_encoding::{DecodePem as _, EncodePem as _};
        use std::io::ErrorKind;
        use tokio::fs::{File, OpenOptions};
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let mut buffer = Vec::new();
        Ok(match self {
            Self::Ed25519 { path } => match File::open(path).await {
                Ok(mut file) => {
                    file.read_to_end(&mut buffer).await?;
                    PrivateKey::decode_pem(&buffer)?
                }
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    let keypair = Ed25519Keypair::random(&mut rand::rng());
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
            Self::Rsa { path } => match File::open(path).await {
                Ok(mut file) => {
                    file.read_to_end(&mut buffer).await?;
                    PrivateKey::decode_pem(&buffer)?
                }
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    let keypair = RsaKeypair::random(&mut rand::rng(), 4096)?;
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

fn deserialize_headers<'de, D>(deserializer: D) -> Result<Vec<HeaderValue>, D::Error>
where
    D: de::Deserializer<'de>,
{
    use de::Error as _;

    <Vec<String>>::deserialize(deserializer)?
        .iter()
        .map(|data| HeaderValue::from_str(data).map_err(D::Error::custom))
        .collect()
}

fn serialize_headers<S>(data: &[HeaderValue], serializer: S) -> Result<S::Ok, S::Error>
where
    S: ser::Serializer,
{
    use ser::Error as _;
    use ser::SerializeSeq as _;

    let mut seq = serializer.serialize_seq(Some(data.len()))?;
    for value in data {
        seq.serialize_element(value.to_str().map_err(S::Error::custom)?)?;
    }
    seq.end()
}
