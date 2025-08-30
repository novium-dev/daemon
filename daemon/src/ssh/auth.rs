/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
use crate::config::SSHConfig;
use crate::{Daemon, DaemonExtra};
use axum::http;
use core::hash::Hash;
use core::time::Duration;
use daemon_common::ssh::DaemonBackendAuthResponse;
use russh::keys::ssh_key::Fingerprint;
use russh::keys::{HashAlg, PublicKey};
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

    daemon
        .client
        .get(url)
        .basic_auth(user, Some(password))
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

    daemon
        .client
        .get(url)
        .basic_auth(
            user,
            Some(hex::encode(
                public_key.fingerprint(HashAlg::Sha512).as_bytes(),
            )),
        )
        .send()
        .await
        .inspect_err(|err| debug!("SSH Authentication failed (publickey): {:?}", err))
        .map_err(io::Error::other)?
        .error_for_status()
        .inspect_err(|err| debug!("SSH Authentication failed (publickey): {:?}", err))
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

impl
    InvalidationCacheMap<(String, [u8; HashAlg::Sha512.digest_size()]), DaemonBackendAuthResponse>
{
    pub async fn get_resolved(
        &mut self,
        daemon: &Daemon<impl DaemonExtra>,
        ssh: &SSHConfig,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<DaemonBackendAuthResponse, io::Error> {
        let key = (
            user.to_owned(),
            fingerprint_to_sha512(public_key.fingerprint(HashAlg::Sha512)), // TODO: Replace key to (String, Fingerprint), already merged in ssh-key but not released and thus not included in internal-russh-forked-ssh-key
        );
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

#[deprecated]
fn fingerprint_to_sha512(fingerprint: Fingerprint) -> [u8; 64] {
    #[expect(
        clippy::unwrap_used,
        reason = "will be fixed, when upstream ssh-key 0.7 releases"
    )]
    fingerprint.sha512().unwrap()
}
