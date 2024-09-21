use axum_server::tls_rustls::RustlsConfig;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use ed25519_dalek::ed25519::KeypairBytes;
use ed25519_dalek::pkcs8::spki::der::pem::LineEnding;
use ed25519_dalek::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use ed25519_dalek::{SigningKey, VerifyingKey};
use openssl::rsa::Rsa;
use rand::rngs::OsRng;
use russh_keys::encoding::Bytes;
use russh_keys::key::{KeyPair, RsaPrivate, SignatureHash};
use serde::de::Error;
use serde::{Deserialize, Deserializer};
use std::io::{self, ErrorKind};
use std::path::PathBuf;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Deserialize)]
pub(crate) struct DaemonConfig {
    pub(crate) identifier: String,
    pub(crate) token: String,
    #[serde(deserialize_with = "deserialize_websocket_key")]
    pub(crate) websocket_key: VerifyingKey,
    pub(crate) websocket_allow_duration: u64,
    pub(crate) origin: String,
    pub(crate) storage_path: String,
    pub(crate) btrfs: bool,
    pub(crate) bind: String,
    pub(crate) ssh_bind: String,
    pub(crate) database: String,
    pub(crate) ssl: SSLConfig,
    pub(crate) ssh_host_key: Vec<SSHHostKey>,
}

fn deserialize_websocket_key<'de, D>(deserializer: D) -> Result<VerifyingKey, D::Error>
where
    D: Deserializer<'de>,
{
    let str = String::deserialize(deserializer)?;
    let mut buffer = Vec::with_capacity(32);
    BASE64_STANDARD
        .decode_vec(str, &mut buffer)
        .map_err(D::Error::custom)?;
    VerifyingKey::from_bytes(buffer.as_slice().try_into().map_err(D::Error::custom)?)
        .map_err(D::Error::custom)
}

#[derive(Deserialize)]
pub(crate) struct SSLConfig {
    pub(crate) enabled: bool,
    pub(crate) private_key: Option<String>,
    pub(crate) certificate: Option<String>,
}

impl SSLConfig {
    pub(crate) async fn as_rustls(&self) -> Result<RustlsConfig, io::Error> {
        RustlsConfig::from_pem_file(
            PathBuf::from(self.certificate.as_ref().unwrap()),
            PathBuf::from(self.private_key.as_ref().unwrap()),
        )
        .await
    }
}

#[derive(Deserialize)]
#[serde(tag = "type")]
pub(crate) enum SSHHostKey {
    #[serde(rename = "ed25519")]
    ED25519 { path: String },
    #[serde(rename = "rsa")]
    #[allow(clippy::upper_case_acronyms)]
    RSA { path: String, hash: SignatureHash },
}

impl SSHHostKey {
    pub(crate) async fn as_russh_key(&self) -> Result<KeyPair, io::Error> {
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
