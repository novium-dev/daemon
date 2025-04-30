use crate::storage::archive::compression::ArchiveCompression;
#[cfg(feature = "ssl")]
use axum_server::tls_rustls::RustlsConfig;
use base64::prelude::*;
use ed25519_dalek::VerifyingKey;
use reqwest::Url;
#[cfg(feature = "ssh")]
use russh::keys::PrivateKey;
#[cfg(feature = "ssh")]
use russh::keys::ssh_key::private::{Ed25519Keypair, RsaKeypair};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::HashMap;
#[cfg(feature = "ssl")]
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct DaemonConfig {
    #[allow(unused)]
    pub(crate) identifier: String,
    pub(crate) token: String,
    pub(crate) backend_url: Url,
    #[serde(deserialize_with = "deserialize_user_key")]
    #[serde(serialize_with = "serialize_user_key")]
    pub(crate) user_key: VerifyingKey,
    pub(crate) origin: String,
    pub(crate) storage: HashMap<String, StorageEntry>,
    pub(crate) backup: HashMap<String, BackupEntry>,
    pub(crate) bind: String,
    pub(crate) database: String,
    #[cfg(feature = "ssl")]
    pub(crate) ssl: SSLConfig,
    #[cfg(feature = "ssh")]
    pub(crate) ssh: SSHConfig,
    #[serde(default)]
    pub(crate) network: NetworkConfig,
    pub(crate) mounts: Vec<MountPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MountPoint {
    pub(crate) src: String,
    pub(crate) dest: String,
    pub(crate) mode: String,
}

#[cfg(feature = "ssh")]
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SSHConfig {
    pub(crate) bind: String,
    pub(crate) host_key: Vec<SSHHostKey>,
    pub(crate) auth_cache_duration: u64,
    pub(crate) auth_url: reqwest::Url,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "method")]
pub(crate) enum StorageEntry {
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
    pub(crate) fn path(&self) -> &String {
        match self {
            StorageEntry::Directory { path } => path,
            #[cfg(feature = "btrfs")]
            StorageEntry::Btrfs { path, .. } => path,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "method")]
pub(crate) enum BackupEntry {
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
    pub(crate) fn path(&self) -> &String {
        match self {
            BackupEntry::Archive { path, .. } => path,
            #[cfg(feature = "btrfs")]
            BackupEntry::Btrfs { path, .. } => path,
        }
    }

    pub(crate) fn order(&self) -> Option<i8> {
        match self {
            BackupEntry::Archive { order, .. } => *order,
            #[cfg(feature = "btrfs")]
            BackupEntry::Btrfs { order, .. } => *order,
        }
    }
}

fn deserialize_user_key<'de, D>(deserializer: D) -> Result<VerifyingKey, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use base64::Engine;
    use base64::prelude::*;
    use serde::de::Error;

    let str = String::deserialize(deserializer)?;
    let mut buffer = Vec::with_capacity(32);
    BASE64_STANDARD
        .decode_vec(str, &mut buffer)
        .map_err(D::Error::custom)?;
    VerifyingKey::from_bytes(buffer.as_slice().try_into().map_err(D::Error::custom)?)
        .map_err(D::Error::custom)
}

fn serialize_user_key<S: Serializer>(key: &VerifyingKey, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&BASE64_STANDARD.encode(key.as_bytes()))
}

#[cfg(feature = "ssl")]
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SSLConfig {
    pub(crate) enabled: bool,
    pub(crate) private_key: Option<String>,
    pub(crate) certificate: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub(crate) struct NetworkConfig {
    // TODO: Assign dedicated ipv6 address to container, see #312
}

#[cfg(feature = "ssl")]
impl SSLConfig {
    pub(crate) async fn as_rustls(&self) -> Result<RustlsConfig, std::io::Error> {
        RustlsConfig::from_pem_file(
            PathBuf::from(self.certificate.as_ref().unwrap()),
            PathBuf::from(self.private_key.as_ref().unwrap()),
        )
        .await
    }
}

#[cfg(feature = "ssh")]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum SSHHostKey {
    #[serde(rename = "ed25519")]
    ED25519 { path: String },
    #[serde(rename = "rsa")]
    #[allow(clippy::upper_case_acronyms)]
    RSA { path: String },
}

#[cfg(feature = "ssh")]
impl SSHHostKey {
    pub(crate) async fn as_russh_key(&self) -> Result<PrivateKey, anyhow::Error> {
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
