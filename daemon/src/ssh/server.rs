// SPDX-License-Identifier: AGPL-3.0-only
use crate::ssh::auth::InvalidationCacheMap;
use crate::ssh::handler::DaemonSSHHandler;
use crate::{Daemon, DaemonExtra};
use core::net::SocketAddr;
use core::time::Duration;
use russh::server::{Handler, Server};
use std::collections::HashMap;
use std::io::ErrorKind;
use tracing::debug;

#[derive(Clone)]
pub struct DaemonSSHServer<T: DaemonExtra> {
    pub(crate) daemon: Daemon<T>,
    pub(crate) cache_invalidation: Duration,
}

impl<T: DaemonExtra> Server for DaemonSSHServer<T> {
    type Handler = DaemonSSHHandler<T>;

    fn new_client(&mut self, _peer_addr: Option<SocketAddr>) -> Self::Handler {
        DaemonSSHHandler {
            daemon: self.daemon.clone(),
            channels: HashMap::new(),
            publickey_cache: InvalidationCacheMap::new(self.cache_invalidation),
            auth: None,
        }
    }

    fn handle_session_error(&mut self, err: <Self::Handler as Handler>::Error) {
        if let russh::Error::IO(ref err) = err
            && matches!(
                err.kind(),
                ErrorKind::UnexpectedEof | ErrorKind::ConnectionReset
            )
        {
            return;
        }
        debug!("Session error: {err:?}");
    }
}
