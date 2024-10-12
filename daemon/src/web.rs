use axum::http::StatusCode;
use diesel::result::Error;
use log::{debug, warn};

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
