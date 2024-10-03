#[cfg(feature = "ssl")]
use axum_server::tls_rustls::RustlsConfig;
#[cfg(all(feature = "websocket", not(feature = "websocket_debug_auth_bypass")))]
use ed25519_dalek::VerifyingKey;
#[cfg(feature = "ssh")]
use russh_keys::encoding::Bytes;
#[cfg(feature = "ssh")]
use russh_keys::key::{KeyPair, RsaPrivate, SignatureHash};
use serde::Deserialize;
#[cfg(feature = "ssl")]
use std::path::PathBuf;

#[derive(Deserialize)]
pub(crate) struct DaemonConfig {
    #[allow(unused)]
    pub(crate) identifier: String,
    pub(crate) token: String,
    #[cfg(all(feature = "websocket", not(feature = "websocket_debug_auth_bypass")))]
    #[serde(deserialize_with = "deserialize_websocket_key")]
    pub(crate) websocket_key: VerifyingKey,
    #[cfg(all(feature = "websocket", not(feature = "websocket_debug_auth_bypass")))]
    pub(crate) websocket_allow_duration: u64,
    #[cfg(feature = "websocket")]
    pub(crate) origin: String,
    pub(crate) storage_path: String,
    #[cfg(feature = "btrfs")]
    pub(crate) btrfs: bool,
    pub(crate) bind: String,
    #[cfg(feature = "ssh")]
    pub(crate) ssh_bind: String,
    pub(crate) database: String,
    #[cfg(feature = "ssl")]
    pub(crate) ssl: SSLConfig,
    #[cfg(feature = "ssh")]
    pub(crate) ssh_host_key: Vec<SSHHostKey>,
    #[cfg(feature = "ssh")]
    pub(crate) ssh_auth_cache_duration: u64,
    #[cfg(feature = "ssh")]
    pub(crate) ssh_auth_url: String,
}

#[cfg(all(feature = "websocket", not(feature = "websocket_debug_auth_bypass")))]
fn deserialize_websocket_key<'de, D>(deserializer: D) -> Result<VerifyingKey, D::Error>
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

#[cfg(feature = "ssl")]
#[derive(Deserialize)]
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
#[derive(Deserialize)]
#[serde(tag = "type")]
pub(crate) enum SSHHostKey {
    #[serde(rename = "ed25519")]
    ED25519 { path: String },
    #[serde(rename = "rsa")]
    #[allow(clippy::upper_case_acronyms)]
    RSA { path: String, hash: SignatureHash },
}

#[cfg(feature = "ssh")]
impl SSHHostKey {
    pub(crate) async fn as_russh_key(&self) -> Result<KeyPair, std::io::Error> {
        use ed25519_dalek::pkcs8::KeypairBytes;
        use ed25519_dalek::pkcs8::{DecodePrivateKey, EncodePrivateKey};
        use ed25519_dalek::SigningKey;
        use openssl::rsa::Rsa;
        use pkcs8::LineEnding;
        use rand::rngs::OsRng;
        use std::io::{self, ErrorKind};
        use tokio::fs::{File, OpenOptions};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut buffer = String::new();
        Ok(match self {
            SSHHostKey::ED25519 { path } => KeyPair::Ed25519(match File::open(path).await {
                Ok(mut file) => {
                    file.read_to_string(&mut buffer).await?;
                    let keypair = KeypairBytes::from_pkcs8_pem(&buffer)
                        .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                    SigningKey::from_bytes(&keypair.secret_key)
                }
                Err(err) if err.kind() == ErrorKind::NotFound => {
                    let mut rng = OsRng;
                    let key = SigningKey::generate(&mut rng);
                    let mut bytes = [0u8; 64];
                    let (sk, pk) = bytes.split_at_mut(32);
                    sk.copy_from_slice(key.as_bytes());
                    pk.copy_from_slice(key.verifying_key().as_bytes());
                    let bytes = KeypairBytes::from_bytes(&bytes)
                        .to_pkcs8_pem(LineEnding::LF)
                        .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                    let mut file = OpenOptions::new()
                        .create(true)
                        .write(true)
                        .truncate(true)
                        .open(path)
                        .await?;
                    file.write_all(bytes.bytes()).await?;
                    key
                }
                Err(err) => {
                    return Err(err);
                }
            }),
            SSHHostKey::RSA { path, hash } => {
                let key = match File::open(path).await {
                    Ok(mut file) => {
                        file.read_to_string(&mut buffer).await?;
                        Rsa::private_key_from_pem(buffer.as_bytes()).unwrap()
                    }
                    Err(err) if err.kind() == ErrorKind::NotFound => {
                        let key = Rsa::generate(4096u32)?;
                        let bytes = key.private_key_to_pem()?;
                        let mut file = OpenOptions::new()
                            .create(true)
                            .write(true)
                            .truncate(true)
                            .open(path)
                            .await?;
                        file.write_all(&bytes).await?;
                        key
                    }
                    Err(err) => return Err(err),
                };
                let key = RsaPrivate::new_from_der(&key.private_key_to_der().unwrap()).unwrap();
                KeyPair::RSA { key, hash: *hash }
            }
        })
    }
}
