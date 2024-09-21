use axum_server::tls_rustls::RustlsConfig;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use ed25519_dalek::ed25519::KeypairBytes;
use ed25519_dalek::pkcs8::DecodePrivateKey;
use ed25519_dalek::{SigningKey, VerifyingKey};
use openssl::rsa::Rsa;
use russh_keys::key::{KeyPair, RsaPrivate, SignatureHash};
use serde::de::Error;
use serde::{Deserialize, Deserializer};
use std::fs::File;
use std::io;
use std::io::Read;
use std::path::PathBuf;

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
    pub(crate) fn as_russh_key(&self) -> KeyPair {
        let mut buffer = String::new();
        match self {
            SSHHostKey::ED25519 { path } => {
                File::open(path)
                    .unwrap()
                    .read_to_string(&mut buffer)
                    .unwrap();

                let keypair = KeypairBytes::from_pkcs8_pem(&buffer).unwrap();
                KeyPair::Ed25519(SigningKey::from_bytes(&keypair.secret_key))
            }
            SSHHostKey::RSA { path, hash } => {
                File::open(path)
                    .unwrap()
                    .read_to_string(&mut buffer)
                    .unwrap();

                let key = Rsa::private_key_from_pem(buffer.as_bytes()).unwrap();
                let key: RsaPrivate =
                    RsaPrivate::new_from_der(&key.private_key_to_der().unwrap()).unwrap();
                KeyPair::RSA { key, hash: *hash }
            }
        }
    }
}
