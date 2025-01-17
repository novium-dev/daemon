use crate::DaemonState;
use axum::http::StatusCode;
use tracing::{debug, warn};
use utoipa_axum::router::OpenApiRouter;

pub(crate) mod backup;
#[cfg(feature = "database")]
pub(crate) mod database;
pub(crate) mod files;
mod network;
#[cfg(feature = "database")]
pub(crate) mod payload;
pub(crate) mod resource;
pub(crate) mod server;
#[cfg(feature = "user")]
pub(crate) mod user;
#[cfg(feature = "user")]
pub(crate) mod websocket;

pub(crate) fn sql_error_to_http(err: sqlx::Error) -> StatusCode {
    if let sqlx::Error::Database(database) = &err {
        if database.is_unique_violation()
            || database.is_check_violation()
            || database.is_foreign_key_violation()
        {
            debug!("Database query failed (violation): {:?}", err);
            return StatusCode::BAD_REQUEST;
        }
    }
    warn!("Database query failed: {:?}", err);
    StatusCode::INTERNAL_SERVER_ERROR
}

pub(crate) fn config() -> OpenApiRouter<DaemonState> {
    let mut router = OpenApiRouter::new()
        .merge(server::config())
        .merge(resource::config())
        .merge(backup::config())
        .merge(files::config())
        .merge(network::config());
    #[cfg(feature = "database")]
    {
        router = router.merge(database::config());
    }
    router
}

#[cfg(feature = "user")]
pub(crate) fn user_config() -> OpenApiRouter<DaemonState> {
    OpenApiRouter::new()
        .merge(websocket::user_config())
        .merge(backup::user_config())
        .merge(files::user_config())
}
