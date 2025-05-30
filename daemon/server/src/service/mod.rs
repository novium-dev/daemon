use crate::Daemon;
use axum::extract::Request;
use axum::{Extension, Router};
use axum_server::Handle;
use daemon_common::framework_router;
use daemon_common::service::backup::BackupService;
use daemon_common::service::database::DatabaseService;
use daemon_common::service::files::FilesService;
use daemon_common::service::network::NetworkService;
use daemon_common::service::resource::ResourceService;
use daemon_common::service::schedule::ScheduleService;
use daemon_common::service::server::ServerService;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

mod backup;
mod database;
#[cfg(feature = "trace_sync")]
mod debug;
mod files;
pub(crate) mod middleware;
mod network;
mod resource;
mod schedule;
mod server;
mod setup;

impl Daemon {
    pub fn router(&self) -> Router {
        #[derive(OpenApi)]
        #[openapi()]
        struct ApiDoc;

        #[allow(unused_mut)]
        let mut routes = framework_router!(Self self {
            ServerService, BackupService, FilesService, NetworkService,
            ResourceService, DatabaseService, ScheduleService
        });

        #[cfg(feature = "trace_sync")]
        {
            use crate::common::MultiRouterExtension;
            routes = routes.merge(
                framework_router!(Self self { daemon_common::service::debug::DebugService }),
            );
        }

        let (routes, routes_user) = routes;

        let cors = tower_http::cors::CorsLayer::new()
            .allow_origin(tower_http::cors::AllowOrigin::exact(
                axum::http::header::HeaderValue::from_str(&self.config.origin).unwrap(),
            ))
            .allow_methods([axum::http::Method::GET])
            .allow_headers(tower_http::cors::AllowHeaders::any());

        let (mut router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
            .merge(
                routes
                    .layer(axum::middleware::from_fn(middleware::middleware))
                    .layer(Extension(self.config.clone())),
            )
            .merge(routes_user.layer(cors))
            .split_for_parts();

        #[cfg(debug_assertions)]
        {
            use utoipa_swagger_ui::SwaggerUi;
            router = router.merge(SwaggerUi::new("/docs").url("/docs/openapi.json", api));
        }

        router = router.layer(TraceLayer::new_for_http());

        #[cfg(feature = "sentry")]
        {
            use sentry::integrations::tower::{NewSentryLayer, SentryHttpLayer};
            router = router
                .layer(NewSentryLayer::<Request>::new_from_top())
                .layer(SentryHttpLayer::new().enable_transaction());
        }
        router
    }

    pub async fn serve_http(&self) -> Result<(), anyhow::Error> {
        let rustls = if let Some(ssl) = &self.config.ssl {
            Some(
                ssl.as_rustls()
                    .await
                    .inspect_err(|_err| error!("Failed to parse ssl certs!"))?,
            )
        } else {
            None
        };
        let handle = Handle::new();
        let mut server_task = self.server_task.lock().await;
        for listener in self.config.listeners.clone() {
            let handle = handle.clone();
            let router = self.router();
            let rustls = rustls.clone();
            server_task.spawn(async move {
                let _ = if let Some(rustls) = rustls {
                    axum_server::bind_rustls(listener, rustls)
                        .handle(handle)
                        .serve(router.into_make_service())
                        .await
                        .inspect_err(|_err| error!("Failed to serve https server!"))
                } else {
                    axum_server::bind(listener)
                        .handle(handle)
                        .serve(router.into_make_service())
                        .await
                        .inspect_err(|_err| error!("Failed to serve http server!"))
                };
            });
        }
        let mut recv = self.shutdown_channel.subscribe();
        tokio::task::spawn(async move {
            loop {
                match recv.recv().await {
                    Err(RecvError::Lagged(_)) => {}
                    Ok(_) | Err(RecvError::Closed) => {
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
