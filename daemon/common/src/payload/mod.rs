#[cfg(feature = "database")]
pub mod database;
#[cfg(feature = "files")]
pub mod files;
pub mod resource;
pub mod server;

#[cfg(target_arch = "wasm32")]
pub use wasm_bindgen::prelude::*;

use serde::{Deserialize, Serialize};

#[cfg(target_arch = "wasm32")]
pub use tsify::Tsify;

#[derive(Serialize, Deserialize)]
pub struct IdResponse {
    pub id: u32,
}

#[derive(Serialize, Deserialize)]
pub struct TimeIdResponse {
    pub id: i64,
}
