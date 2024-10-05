#[cfg(target_arch = "wasm32")]
use crate::payload::Tsify;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[cfg_attr(target_arch = "wasm32", derive(Tsify))]
#[cfg_attr(target_arch = "wasm32", tsify(into_wasm_abi, from_wasm_abi))]
pub struct PublicServerState {
    pub label: String,
    pub start: bool,
    pub stop: bool,
    pub kill: bool,
}

#[cfg(target_arch = "wasm32")]
impl Into<wasm_bindgen::JsValue> for PublicServerState {
    fn into(self) -> wasm_bindgen::JsValue {
        self.into_js().unwrap().obj
    }
}
