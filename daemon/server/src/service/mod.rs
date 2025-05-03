use crate::{Daemon, web};
use axum::extract::Request;
use axum::{Extension, Router};
use daemon_common::framework_router;
use daemon_common::service::backup::BackupService;
use daemon_common::service::database::DatabaseService;
use daemon_common::service::files::FilesService;
use daemon_common::service::network::NetworkService;
use daemon_common::service::resource::ResourceService;
use daemon_common::service::schedule::ScheduleService;
use daemon_common::service::server::ServerService;
use std::io;
use tower_http::trace::TraceLayer;
use tracing::error;
use utoipa::OpenApi;
use utoipa_axum::router::OpenApiRouter;

mod backup;
mod database;
mod files;
pub(crate) mod middleware;
mod network;
mod resource;
mod schedule;
mod server;

impl Daemon {
    pub fn router(&self) -> Router {
        #[derive(OpenApi)]
        #[openapi()]
        struct ApiDoc;

        let (routes, routes_user) = framework_router!(Self self {
            ServerService, BackupService, FilesService, NetworkService,
            ResourceService, DatabaseService, ScheduleService
        });

        let cors = tower_http::cors::CorsLayer::new()
            .allow_origin(tower_http::cors::AllowOrigin::exact(
                axum::http::header::HeaderValue::from_str(&self.config.origin).unwrap(),
            ))
            .allow_methods([axum::http::Method::GET])
            .allow_headers(tower_http::cors::AllowHeaders::any());

        let mut router = OpenApiRouter::with_openapi(ApiDoc::openapi())
            .merge(routes)
            .layer(axum::middleware::from_fn(middleware::middleware))
            .merge(web::config_unauthorized().with_state(self.clone()))
            .layer(Extension(self.clone()));

        router = router.merge(routes_user.layer(cors));

        let (mut router, api) = router.split_for_parts();

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
                .layer(SentryHttpLayer::with_transaction());
        }
        router
    }

    pub async fn serve_http(&self) -> Result<(), io::Error> {
        let rustls = if let Some(ssl) = &self.config.ssl {
            Some(
                ssl.as_rustls()
                    .await
                    .inspect_err(|_err| error!("Failed to parse ssl certs!"))?,
            )
        } else {
            None
        };
        let mut server_task = self.server_task.lock().await;
        for listener in self.config.listeners.clone() {
            let router = self.router();
            let rustls = rustls.clone();
            server_task.spawn(async move {
                let _ = if let Some(rustls) = rustls {
                    axum_server::bind_rustls(listener, rustls)
                        .serve(router.into_make_service())
                        .await
                        .inspect_err(|_err| error!("Failed to serve https server!"))
                } else {
                    axum_server::bind(listener)
                        .serve(router.into_make_service())
                        .await
                        .inspect_err(|_err| error!("Failed to serve http server!"))
                };
            });
        }
        Ok(())
    }
}
