use crate::DaemonState;
use axum::http::StatusCode;
use diesel::result::Error;
use tracing::{debug, warn};
use utoipa_axum::router::OpenApiRouter;

#[cfg(feature = "backups")]
pub(crate) mod backup;
#[cfg(feature = "database")]
pub(crate) mod database;
#[cfg(feature = "files")]
pub(crate) mod files;
#[cfg(feature = "database")]
pub(crate) mod payload;
pub(crate) mod resource;
pub(crate) mod server;
#[cfg(feature = "user")]
pub(crate) mod user;
#[cfg(feature = "websocket")]
pub(crate) mod websocket;

pub(crate) fn diesel_error_to_http(diesel: diesel::result::Error) -> StatusCode {
    match diesel {
        Error::NotFound => {
            debug!("Database query failed: {:?}", diesel);
            StatusCode::NOT_FOUND
        }
        _ => {
            warn!("Database query failed: {:?}", diesel);
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

pub(crate) fn config() -> OpenApiRouter<DaemonState> {
    let mut router = OpenApiRouter::new()
        .merge(server::config())
        .merge(resource::config());
    #[cfg(feature = "database")]
    {
        router = router.merge(database::config());
    }
    #[cfg(feature = "backups")]
    {
        router = router.merge(backup::config());
    }
    #[cfg(feature = "files")]
    {
        router = router.merge(files::config());
    }
    router
}

pub(crate) fn user_config() -> OpenApiRouter<DaemonState> {
    let mut router = OpenApiRouter::new();
    #[cfg(feature = "user")]
    {
        #[cfg(feature = "websocket")]
        {
            router = router.merge(websocket::user_config())
        }
        #[cfg(feature = "backups")]
        {
            router = router.merge(backup::user_config());
        }
        #[cfg(feature = "files")]
        {
            router = router.merge(files::user_config());
        }
    }
    router
}
