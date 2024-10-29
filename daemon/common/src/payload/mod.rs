pub mod backup;
#[cfg(feature = "database")]
pub mod database;
#[cfg(feature = "files")]
pub mod files;
pub mod resource;
pub mod server;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct IdResponse {
    pub id: u32,
}

#[derive(Serialize, Deserialize)]
pub struct TimeIdResponse {
    pub time: i64,
}
