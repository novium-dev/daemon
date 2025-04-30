use crate::Daemon;
use axum::http::StatusCode;
use tracing::{debug, warn};
use utoipa_axum::router::OpenApiRouter;

#[cfg(feature = "database")]
pub(crate) mod database;
pub(crate) mod files;
mod network;
#[cfg(feature = "database")]
pub(crate) mod payload;
pub(crate) mod resource;
pub(crate) mod websocket;

mod schedule;
#[cfg(feature = "trace_sync")]
pub(crate) mod trace;

#[deprecated]
pub(crate) mod user;

#[deprecated]
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

pub(crate) fn config() -> OpenApiRouter<Daemon> {
    #[allow(unused_mut)]
    let mut router = OpenApiRouter::new()
        .merge(resource::config())
        .merge(files::config())
        .merge(network::config())
        .merge(schedule::config());
    #[cfg(feature = "database")]
    {
        router = router.merge(database::config());
    }
    router
}

pub(crate) fn user_config() -> OpenApiRouter<Daemon> {
    OpenApiRouter::new()
        .merge(websocket::user_config())
        .merge(files::user_config())
}

pub(crate) fn config_unauthorized() -> OpenApiRouter<Daemon> {
    #[allow(unused_mut)]
    let mut router = OpenApiRouter::new();
    #[cfg(feature = "trace_sync")]
    {
        router = router.merge(trace::config());
    }
    router
}
