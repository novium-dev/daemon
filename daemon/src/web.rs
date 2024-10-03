use axum::http::StatusCode;
use diesel::result::Error;
use log::{debug, warn};

#[cfg(feature = "database")]
pub(crate) mod database;
#[cfg(feature = "files")]
pub(crate) mod files;
pub(crate) mod payload;
pub(crate) mod server;
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
