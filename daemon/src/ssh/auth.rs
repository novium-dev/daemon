// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::SSHConfig;
use crate::{Daemon, DaemonExtra};
use axum::http;
use base64::Engine as _;
use base64::prelude::BASE64_STANDARD;
use core::hash::Hash;
use core::time::Duration;
use daemon_common::ssh::DaemonBackendAuthResponse;
use reqwest::RequestBuilder;
use russh::keys::ssh_key::Fingerprint;
use russh::keys::{HashAlg, PublicKey};
use rustls::SignatureScheme;
use std::collections::HashMap;
use std::io;
use std::io::ErrorKind;
use std::time::Instant;
use tracing::debug;

pub struct InvalidationCacheMap<T: Clone + Hash + Eq, E: Clone>(HashMap<T, (Instant, E)>, Duration);

impl<T: Clone + Hash + Eq, E: Clone> InvalidationCacheMap<T, E> {
    pub fn new(invalidation: Duration) -> Self {
        Self(HashMap::new(), invalidation)
    }

    pub fn insert(&mut self, key: T, val: E) {
        self.update();
        self.0.insert(key, (Instant::now(), val));
    }

    pub fn get(&mut self, key: &T) -> Option<&mut E> {
        self.update();
        self.0.get_mut(key).map(|(_time, value)| value)
    }

    pub fn update(&mut self) {
        for (key, (time, _val)) in self.0.clone() {
            if time.elapsed() >= self.1 {
                self.0.remove(&key);
            }
        }
    }
}

const SIGNATURE_CONTEXT: &str = "novium-node-auth";

const SIGNATURE_SCHEMES: &[SignatureScheme] = &[
    SignatureScheme::ECDSA_NISTP256_SHA256,
    SignatureScheme::ECDSA_NISTP384_SHA384,
    SignatureScheme::ED25519,
    SignatureScheme::RSA_PSS_SHA256,
    SignatureScheme::RSA_PKCS1_SHA256,
];

/// Attaches the basic auth credentials and signs them with the node key, so the
/// backend can tell the request comes from this node.
fn signed_basic_auth(
    daemon: &Daemon<impl DaemonExtra>,
    request: RequestBuilder,
    user: &str,
    password: &str,
) -> Result<RequestBuilder, io::Error> {
    let authorization = format!(
        "Basic {}",
        BASE64_STANDARD.encode(format!("{user}:{password}"))
    );
    let timestamp = chrono::Utc::now().timestamp();
    let message = format!("{SIGNATURE_CONTEXT}\n{timestamp}\n{authorization}");

    let signer = daemon
        .key
        .tls_cert
        .key
        .choose_scheme(SIGNATURE_SCHEMES)
        .ok_or_else(|| io::Error::other("the node key supports no known signature scheme"))?;
    let signature = signer.sign(message.as_bytes()).map_err(io::Error::other)?;
    let certificate = daemon
        .key
        .cert
        .first()
        .ok_or_else(|| io::Error::other("the node key has no certificate"))?;

    Ok(request
        .header(http::header::AUTHORIZATION, authorization)
        .header(
            "x-novium-node-certificate",
            BASE64_STANDARD.encode(certificate),
        )
        .header("x-novium-node-signature", BASE64_STANDARD.encode(signature))
        .header(
            "x-novium-node-signature-scheme",
            u16::from(signer.scheme()).to_string(),
        )
        .header("x-novium-node-timestamp", timestamp.to_string()))
}

pub async fn password_auth(
    daemon: &Daemon<impl DaemonExtra>,
    ssh: &SSHConfig,
    user: &str,
    password: &str,
) -> Result<DaemonBackendAuthResponse, io::Error> {
    let mut url = ssh
        .auth_url
        .join("/api/internal/auth/password")
        .map_err(io::Error::other)?;
    url.query_pairs_mut()
        .append_pair("node", &daemon.key.id.to_string());

    signed_basic_auth(daemon, daemon.client.get(url), user, password)?
        .send()
        .await
        .inspect_err(|err| debug!("SSH Authentication failed (password): {err:?}"))
        .map_err(|err| match err.status() {
            Some(http::StatusCode::UNAUTHORIZED) => {
                io::Error::new(ErrorKind::PermissionDenied, err)
            }
            _ => io::Error::other(err),
        })?
        .json()
        .await
        .map_err(io::Error::other)
}

pub async fn publickey_auth(
    daemon: &Daemon<impl DaemonExtra>,
    ssh: &SSHConfig,
    user: &str,
    public_key: &PublicKey,
) -> Result<DaemonBackendAuthResponse, io::Error> {
    let mut url = ssh
        .auth_url
        .join("/api/internal/auth/publickey")
        .map_err(io::Error::other)?;
    url.query_pairs_mut()
        .append_pair("node", &daemon.key.id.to_string());

    signed_basic_auth(
        daemon,
        daemon.client.get(url),
        user,
        &hex::encode(public_key.fingerprint(HashAlg::Sha512).as_bytes()),
    )?
    .send()
    .await
    .inspect_err(|err| debug!("SSH Authentication failed (publickey): {:?}", err))
    .map_err(io::Error::other)?
    .error_for_status()
    .inspect_err(|err| debug!("SSH Authentication failed (publickey): {:?}", err))
    .map_err(|err| match err.status() {
        Some(http::StatusCode::UNAUTHORIZED) => io::Error::new(ErrorKind::PermissionDenied, err),
        _ => io::Error::other(err),
    })?
    .json()
    .await
    .map_err(io::Error::other)
}

impl InvalidationCacheMap<(String, Fingerprint), DaemonBackendAuthResponse> {
    pub async fn get_resolved(
        &mut self,
        daemon: &Daemon<impl DaemonExtra>,
        ssh: &SSHConfig,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<DaemonBackendAuthResponse, io::Error> {
        let key = (user.to_owned(), public_key.fingerprint(HashAlg::Sha512));
        match self.get(&key) {
            None => {
                let response = publickey_auth(daemon, ssh, user, public_key).await?;
                self.insert(key, response.clone());
                Ok(response)
            }
            Some(response) => Ok(response.clone()),
        }
    }
}
