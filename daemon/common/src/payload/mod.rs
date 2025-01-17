pub mod backup;
#[cfg(feature = "database")]
pub mod database;
pub mod files;
pub mod network;
pub mod node;
pub mod resource;
pub mod server;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({"id": 1}))]
pub struct IdResponse {
    pub id: u32,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({"time": 1731160477597i64}))]
pub struct TimeIdResponse {
    pub time: i64,
}
