#[cfg(target_arch = "wasm32")]
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[cfg_attr(target_arch = "wasm32", derive(Tsify))]
#[cfg_attr(target_arch = "wasm32", tsify(into_wasm_abi, from_wasm_abi))]
pub struct BackupResponseEntry {
    pub id: String,
    pub storage: String,
    pub time: String,
}

#[derive(Serialize, Deserialize)]
pub struct CompressionQuery {
    pub compression: String,
}

#[derive(Serialize, Deserialize)]
pub struct UserCompressionQuery {
    pub timestamp: String,
    pub verify: String,
    pub compression: String,
}

#[cfg(target_arch = "wasm32")]
impl Into<wasm_bindgen::JsValue> for BackupResponseEntry {
    fn into(self) -> wasm_bindgen::JsValue {
        self.into_js().unwrap().obj
    }
}
