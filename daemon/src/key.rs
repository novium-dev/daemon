// SPDX-License-Identifier: AGPL-3.0-only
use core::fmt::{Debug, Formatter};
use daemon_common::key::DaemonKey;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use serde::Serializer as _;
use std::sync::Arc;
use tracing::debug;
use uuid::Uuid;

pub struct DaemonCertResolver(pub Arc<DaemonKey>, pub Option<Arc<CertifiedKey>>, pub Uuid);

impl ResolvesServerCert for DaemonCertResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        debug!("Server name: {:?}", client_hello.server_name());
        debug!("Expected node name: node-{}", self.2);
        match (&self.1, client_hello.server_name()) {
            // No public certificate configured, so everything gets the node one.
            (None, _) => Some(Arc::clone(&self.0.tls_cert)),
            // The backend connects directly by address and pins the node name
            // through its certificate verifier rather than through SNI, so an
            // absent server name means the node certificate is wanted.
            (Some(_), None) => Some(Arc::clone(&self.0.tls_cert)),
            (Some(_), Some(name)) if name == format!("node-{}", self.2) => {
                Some(Arc::clone(&self.0.tls_cert))
            }
            (Some(key), Some(_)) => Some(Arc::clone(key)),
        }
    }
}

impl Debug for DaemonCertResolver {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> core::fmt::Result {
        formatter.serialize_str("DaemonCertResolver")
    }
}
