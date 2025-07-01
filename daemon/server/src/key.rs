use axum::Extension;
use axum::middleware::AddExtension;
use axum_server::accept::Accept;
use axum_server::tls_rustls::RustlsAcceptor;
use core::fmt::{Debug, Formatter};
use daemon_common::key::DaemonKey;
use futures_util::future::BoxFuture;
use rustls::pki_types::CertificateDer;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use serde::Serializer as _;
use std::io;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_rustls::server::TlsStream;
use tower_layer::Layer as _;
use tracing::debug;
use uuid::Uuid;

pub struct DaemonCertResolver(pub Arc<DaemonKey>, pub Option<Arc<CertifiedKey>>, pub Uuid);

impl ResolvesServerCert for DaemonCertResolver {
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        debug!("Server name: {:?}", client_hello.server_name());
        match &self.1 {
            None => Some(Arc::clone(&self.0.tls_cert)),
            Some(_)
                if client_hello
                    .server_name()
                    .eq(&Some(&format!("node-{}", self.2))) =>
            {
                Some(Arc::clone(&self.0.tls_cert))
            }
            Some(key) => Some(Arc::clone(key)),
        }
    }
}

impl Debug for DaemonCertResolver {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> core::fmt::Result {
        formatter.serialize_str("DaemonCertResolver")
    }
}

#[derive(Debug, Clone)]
pub struct TlsData {
    pub peer_certificates: Option<Vec<CertificateDer<'static>>>,
}

#[derive(Debug, Clone)]
pub struct TlsAcceptor<A>(pub RustlsAcceptor<A>);

impl<A, I, S> Accept<I, S> for TlsAcceptor<A>
where
    A: Accept<I, S> + Clone + Send + 'static,
    A::Stream: Send + AsyncRead + AsyncWrite + Unpin,
    A::Service: Send,
    A::Future: Send,
    I: Send + 'static,
    S: Send + 'static,
{
    type Stream = TlsStream<A::Stream>;
    type Service = AddExtension<A::Service, TlsData>;
    type Future = BoxFuture<'static, io::Result<(Self::Stream, Self::Service)>>;

    fn accept(&self, stream: I, service: S) -> Self::Future {
        let acceptor = self.0.clone();

        Box::pin(async move {
            let (stream, service) = acceptor.accept(stream, service).await?;
            let server_conn = stream.get_ref().1;
            let tls_data = TlsData {
                peer_certificates: server_conn.peer_certificates().map(From::from),
            };
            let service = Extension(tls_data).layer(service);

            Ok((stream, service))
        })
    }
}
