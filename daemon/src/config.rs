#[cfg(feature = "ssl")]
use axum_server::tls_rustls::RustlsConfig;
#[cfg(feature = "user")]
use base64::prelude::*;
#[cfg(feature = "user")]
use ed25519_dalek::VerifyingKey;
use reqwest::Url;
#[cfg(feature = "ssh")]
use russh::keys::ssh_key::private::{Ed25519Keypair, RsaKeypair};
#[cfg(feature = "ssh")]
use russh::keys::PrivateKey;
use serde::de::{Error, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use std::collections::HashMap;
use std::fmt::Formatter;
#[cfg(feature = "ssl")]
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct DaemonConfig {
    #[allow(unused)]
    pub(crate) identifier: String,
    pub(crate) token: String,
    pub(crate) backend_url: Url,
    #[cfg(feature = "user")]
    #[serde(deserialize_with = "deserialize_user_key")]
    #[serde(serialize_with = "serialize_user_key")]
    pub(crate) user_key: VerifyingKey,
    #[cfg(feature = "user")]
    #[allow(unused)]
    pub(crate) user_allow_duration: u64,
    #[cfg(feature = "user")]
    pub(crate) origin: String,
    pub(crate) storage: HashMap<String, StorageEntry>,
    pub(crate) backup: HashMap<String, BackupEntry>,
    pub(crate) bind: String,
    pub(crate) database: String,
    #[cfg(feature = "ssl")]
    pub(crate) ssl: SSLConfig,
    #[cfg(feature = "ssh")]
    pub(crate) ssh: SSHConfig,
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

#[derive(Serialize, Deserialize, Copy, Clone)]
#[serde(tag = "type")]
pub(crate) enum ArchiveCompression {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "zstd")]
    Zstd {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: async_compression::Level,
    },
    #[serde(rename = "xz")]
    Xz {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: async_compression::Level,
    },
    #[serde(rename = "zlib")]
    Zlib {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: async_compression::Level,
    },
    #[serde(rename = "gzip")]
    Gzip {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: async_compression::Level,
    },
    #[serde(rename = "lzma")]
    Lzma {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: async_compression::Level,
    },
    #[serde(rename = "brotli")]
    Brotli {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: async_compression::Level,
    },
    #[serde(rename = "bzip2")]
    Bzip2 {
        #[serde(deserialize_with = "deserialize_compression_level")]
        #[serde(serialize_with = "serialize_compression_level")]
        #[serde(default = "default_level")]
        level: async_compression::Level,
    },
}

impl ArchiveCompression {
    #[allow(unused)]
    pub(crate) fn create_decoder<R: tokio::io::AsyncBufRead + Send + Sync + 'static>(
        &self,
        read: R,
    ) -> std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send + Sync + 'static>> {
        use async_compression::tokio::bufread::*;
        match self {
            ArchiveCompression::None => Box::pin(read),
            ArchiveCompression::Zstd { .. } => Box::pin(ZstdDecoder::new(read)),
            ArchiveCompression::Xz { .. } => Box::pin(XzDecoder::new(read)),
            ArchiveCompression::Zlib { .. } => Box::pin(ZlibDecoder::new(read)),
            ArchiveCompression::Gzip { .. } => Box::pin(GzipDecoder::new(read)),
            ArchiveCompression::Lzma { .. } => Box::pin(LzmaDecoder::new(read)),
            ArchiveCompression::Brotli { .. } => Box::pin(BrotliDecoder::new(read)),
            ArchiveCompression::Bzip2 { .. } => Box::pin(BzDecoder::new(read)),
        }
    }

    pub(crate) fn create_encoder<W: tokio::io::AsyncWrite + Send + Sync + 'static>(
        &self,
        write: W,
    ) -> std::pin::Pin<Box<dyn tokio::io::AsyncWrite + Send + Sync + 'static>> {
        use async_compression::tokio::write::*;
        match self {
            ArchiveCompression::None => Box::pin(write),
            ArchiveCompression::Zstd { level } => {
                Box::pin(ZstdEncoder::with_quality(write, *level))
            }
            ArchiveCompression::Xz { level } => Box::pin(XzEncoder::with_quality(write, *level)),
            ArchiveCompression::Zlib { level } => {
                Box::pin(ZlibEncoder::with_quality(write, *level))
            }
            ArchiveCompression::Gzip { level } => {
                Box::pin(GzipEncoder::with_quality(write, *level))
            }
            ArchiveCompression::Lzma { level } => {
                Box::pin(LzmaEncoder::with_quality(write, *level))
            }
            ArchiveCompression::Brotli { level } => {
                Box::pin(BrotliEncoder::with_quality(write, *level))
            }
            ArchiveCompression::Bzip2 { level } => Box::pin(BzEncoder::with_quality(write, *level)),
        }
    }

    pub(crate) fn ext(&self) -> &'static str {
        match self {
            ArchiveCompression::None => "",
            ArchiveCompression::Zstd { .. } => ".zst",
            ArchiveCompression::Xz { .. } => ".xz",
            ArchiveCompression::Zlib { .. } => ".zz",
            ArchiveCompression::Gzip { .. } => ".gz",
            ArchiveCompression::Lzma { .. } => ".lzma",
            ArchiveCompression::Brotli { .. } => ".br",
            ArchiveCompression::Bzip2 { .. } => ".bz2",
        }
    }
}

fn default_level() -> async_compression::Level {
    async_compression::Level::Default
}

fn serialize_compression_level<S: Serializer>(
    v: &async_compression::Level,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use async_compression::Level;
    match *v {
        Level::Fastest => serializer.serialize_str("fastest"),
        Level::Best => serializer.serialize_str("default"),
        Level::Default => serializer.serialize_str("best"),
        Level::Precise(level) => serializer.serialize_i32(level),
        _ => unreachable!(),
    }
}

fn deserialize_compression_level<'de, D>(
    deserializer: D,
) -> Result<async_compression::Level, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use async_compression::Level;
    struct CompressionLevelVisitor;
    impl Visitor<'_> for CompressionLevelVisitor {
        type Value = Level;

        fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
            write!(
                formatter,
                "an integer or \"fastest\", \"default\", \"best\""
            )
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: Error,
        {
            Ok(Level::Precise(v as i32))
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: Error,
        {
            Ok(Level::Precise(v as i32))
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: Error,
        {
            match v {
                "fastest" => Ok(Level::Fastest),
                "default" => Ok(Level::Default),
                "best" => Ok(Level::Best),
                _ => Err(E::custom("Invalid compression level")),
            }
        }
    }
    let visitor = CompressionLevelVisitor;
    deserializer.deserialize_any(visitor)
}

#[cfg(feature = "user")]
fn deserialize_user_key<'de, D>(deserializer: D) -> Result<VerifyingKey, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use base64::prelude::*;
    use base64::Engine;
    use serde::de::Error;

    let str = String::deserialize(deserializer)?;
    let mut buffer = Vec::with_capacity(32);
    BASE64_STANDARD
        .decode_vec(str, &mut buffer)
        .map_err(D::Error::custom)?;
    VerifyingKey::from_bytes(buffer.as_slice().try_into().map_err(D::Error::custom)?)
        .map_err(D::Error::custom)
}

#[cfg(feature = "user")]
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
