// SPDX-License-Identifier: AGPL-3.0-only
use crate::key::DaemonCertResolver;
use crate::{Daemon, DaemonExtra};
use axum::Router;
use axum_server::tls_rustls::{RustlsAcceptor, RustlsConfig};
use axum_server::{Handle, Server};
use core::time::Duration;
use daemon_common::service::backup::BackupService;
use daemon_common::service::database::DatabaseService;
use daemon_common::service::files::FilesService;
use daemon_common::service::framework_router;
use daemon_common::service::network::NetworkService;
use daemon_common::service::resource::ResourceService;
use daemon_common::service::schedule::ScheduleService;
use daemon_common::service::server::ServerService;
use daemon_common::service::setup::SetupService;
use rustls::ServerConfig;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;
use tower_http::compression::CompressionLayer;
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

pub mod attest;
mod backup;
mod database;
mod files;
pub mod middleware;
mod network;
mod resource;
mod schedule;
mod server;
mod setup;

impl<T: DaemonExtra> Daemon<T> {
    pub fn router(&self) -> Router {
        #[derive(OpenApi)]
        #[openapi()]
        struct ApiDoc;

        let (router, router_user) = framework_router!(Self self (with_auth,without_auth,) {
            ServerService, BackupService, FilesService, NetworkService,
            ResourceService, DatabaseService, ScheduleService, SetupService
        });
        let router = router.into_openapi();
        let router_user = router_user.into_openapi();

        let cors = tower_http::cors::CorsLayer::new()
            .allow_origin(tower_http::cors::AllowOrigin::list(
                self.config.origins.iter().cloned(),
            ))
            .allow_methods([axum::http::Method::GET, axum::http::Method::PUT])
            .allow_headers(tower_http::cors::AllowHeaders::any());

        let compression = CompressionLayer::new().gzip(true).zstd(true).br(true);

        let auth = middleware::AuthData {
            cert_store: Arc::clone(&self.root.cert_store),
            node: self.key.id,
        };

        let (mut router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
            .merge(
                router
                    .layer(axum::middleware::from_fn(middleware::middleware))
                    .layer(axum::Extension(auth)),
            )
            .merge(router_user.layer(cors))
            .layer(compression)
            .split_for_parts();

        router = router.merge(attest::router(Arc::clone(&self.key)));

        #[cfg(debug_assertions)]
        {
            use utoipa_swagger_ui::SwaggerUi;
            router = router.merge(SwaggerUi::new("/docs").url("/docs/openapi.json", api));
        };
        #[cfg(not(debug_assertions))]
        let _ = api;

        router = router.layer(TraceLayer::new_for_http());

        #[cfg(feature = "sentry")]
        {
            use sentry::integrations::tower::{NewSentryLayer, SentryHttpLayer};
            router = router
                .layer(NewSentryLayer::<axum::extract::Request>::new_from_top())
                .layer(SentryHttpLayer::new().enable_transaction());
        };
        router
    }

    #[expect(
        clippy::significant_drop_tightening,
        reason = "only runs once on start"
    )]
    pub async fn serve_http(&self) -> Result<(), anyhow::Error> {
        let tls = if let Some(tls) = &self.config.tls {
            Some(Arc::new(tls.load_certs().await?))
        } else {
            None
        };

        // Client authentication happens at the HTTP layer now, so the handshake
        // no longer asks for a certificate. See `service::middleware`.
        let mut rustls = ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(DaemonCertResolver(
                Arc::clone(&self.key),
                tls,
                self.key.id,
            )));
        rustls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        let rustls = RustlsConfig::from_config(Arc::new(rustls));

        let handle = Handle::new();
        let mut server_task = self.server_task.lock().await;
        for listener in self.config.listeners.clone() {
            let handle = handle.clone();
            let router = self.router();
            let rustls = rustls.clone();
            server_task.spawn(async move {
                let _ = Server::bind(listener)
                    .acceptor(RustlsAcceptor::new(rustls.clone()))
                    .handle(handle)
                    .serve(router.into_make_service())
                    .await
                    .inspect_err(|_err| error!("Failed to serve https server!"));
            });
        }
        let mut recv = self.shutdown_channel.subscribe();
        tokio::task::spawn(async move {
            loop {
                match recv.recv().await {
                    Err(RecvError::Lagged(_)) => {}
                    Ok(()) | Err(RecvError::Closed) => {
                        handle.graceful_shutdown(Some(Duration::from_secs(30)));
                        loop {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            let count = handle.connection_count();
                            if count == 0 {
                                break;
                            }
                            info!("HTTP Connections still alive: {}", count);
                        }
                        break;
                    }
                }
            }
        });
        Ok(())
    }
}
