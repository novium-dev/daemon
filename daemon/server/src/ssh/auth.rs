use crate::Daemon;
use axum::http;
use daemon_common::ssh::DaemonBackendAuthResponse;
use russh::keys::{HashAlg, PublicKey};
use std::collections::HashMap;
use std::hash::Hash;
use std::io;
use std::io::ErrorKind;
use std::time::{Duration, Instant};
use tracing::debug;

pub(crate) struct InvalidationCacheMap<T: Clone + Hash + Eq, E: Clone>(
    HashMap<T, (Instant, E)>,
    Duration,
);

#[allow(unused)]
impl<T: Clone + Hash + Eq, E: Clone> InvalidationCacheMap<T, E> {
    pub(crate) fn new(invalidation: Duration) -> Self {
        Self(HashMap::new(), invalidation)
    }

    pub(crate) fn insert(&mut self, key: T, val: E) {
        self.update();
        self.0.insert(key, (Instant::now(), val));
    }

    pub(crate) fn get(&mut self, key: &T) -> Option<&mut E> {
        self.update();
        self.0.get_mut(key).map(|t| &mut t.1)
    }

    pub(crate) fn remove(&mut self, key: &T) {
        self.0.remove(key);
    }

    pub(crate) fn update(&mut self) {
        for (key, (time, _val)) in self.0.clone() {
            if time.elapsed() >= self.1 {
                self.0.remove(&key);
            }
        }
    }
}

pub(crate) async fn password_auth(
    state: &Daemon,
    user: &str,
    password: &str,
) -> Result<DaemonBackendAuthResponse, io::Error> {
    let mut url = state
        .config
        .ssh
        .auth_url
        .join("/api/internal/auth/password")
        .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
    url.query_pairs_mut()
        .append_pair("node", &state.config.identifier);

    state
        .client
        .get(url)
        .basic_auth(user, Some(password))
        .send()
        .await
        .inspect_err(|err| debug!("SSH Authentication failed (password): {:?}", err))
        .map_err(|err| match err {
            err if matches!(err.status(), Some(http::StatusCode::UNAUTHORIZED)) => {
                io::Error::new(ErrorKind::PermissionDenied, err)
            }
            err => io::Error::new(ErrorKind::Other, err),
        })?
        .json()
        .await
        .map_err(|err| io::Error::new(ErrorKind::Other, err))
}

pub(crate) async fn publickey_auth(
    state: &Daemon,
    user: &str,
    public_key: &PublicKey,
) -> Result<DaemonBackendAuthResponse, io::Error> {
    let mut url = state
        .config
        .ssh
        .auth_url
        .join("/api/internal/auth/publickey")
        .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
    url.query_pairs_mut()
        .append_pair("node", &state.config.identifier);

    state
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
        .map_err(|err| io::Error::new(ErrorKind::Other, err))?
        .error_for_status()
        .inspect_err(|err| debug!("SSH Authentication failed (publickey): {:?}", err))
        .map_err(|err| match err {
            err if matches!(err.status(), Some(http::StatusCode::UNAUTHORIZED)) => {
                io::Error::new(ErrorKind::PermissionDenied, err)
            }
            err => io::Error::new(ErrorKind::Other, err),
        })?
        .json()
        .await
        .map_err(|err| io::Error::new(ErrorKind::Other, err))
}

impl
    InvalidationCacheMap<(String, [u8; HashAlg::Sha512.digest_size()]), DaemonBackendAuthResponse>
{
    pub(crate) async fn get_resolved(
        &mut self,
        state: &Daemon,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<DaemonBackendAuthResponse, io::Error> {
        match self.get(&(
            user.to_string(),
            public_key.fingerprint(HashAlg::Sha512).sha512().unwrap(),
        )) {
            None => publickey_auth(state, user, public_key).await,
            Some(response) => Ok(response.clone()),
        }
    }
}
