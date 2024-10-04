#[cfg(feature = "database")]
pub mod database;
#[cfg(feature = "files")]
pub mod files;
pub mod server;

use serde::Serialize;

#[derive(Serialize)]
pub struct IdResponse {
    pub id: u32,
}
