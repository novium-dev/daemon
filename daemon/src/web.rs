use axum::http::StatusCode;
use diesel::result::Error;
use log::warn;

#[cfg(feature = "database")]
pub(crate) mod database;
pub(crate) mod payload;
pub(crate) mod server;
#[cfg(feature = "websocket")]
pub(crate) mod websocket;

pub(crate) fn diesel_error_to_http(diesel: diesel::result::Error) -> StatusCode {
    warn!("Database query failed: {:?}", diesel);
    match diesel {
        Error::NotFound => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
