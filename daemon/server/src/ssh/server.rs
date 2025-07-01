use crate::Daemon;
use crate::ssh::auth::InvalidationCacheMap;
use crate::ssh::handler::DaemonSSHHandler;
use core::net::SocketAddr;
use core::time::Duration;
use russh::server::{Handler, Server};
use std::collections::HashMap;
use std::io::ErrorKind;
use tracing::debug;

#[derive(Clone)]
pub struct DaemonSSHServer {
    pub(crate) daemon: Daemon,
    pub(crate) cache_invalidation: Duration,
}

impl Server for DaemonSSHServer {
    type Handler = DaemonSSHHandler;

    fn new_client(&mut self, _peer_addr: Option<SocketAddr>) -> Self::Handler {
        DaemonSSHHandler {
            daemon: self.daemon.clone(),
            channels: HashMap::new(),
            publickey_cache: InvalidationCacheMap::new(self.cache_invalidation),
            auth: None,
        }
    }

    fn handle_session_error(&mut self, error: <Self::Handler as Handler>::Error) {
        if let russh::Error::IO(ref err) = error {
            if matches!(
                err.kind(),
                ErrorKind::UnexpectedEof | ErrorKind::ConnectionReset
            ) {
                return;
            }
        }
        debug!("Session error: {:?}", error);
    }
}
