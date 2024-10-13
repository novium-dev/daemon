#[cfg(target_arch = "wasm32")]
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq)]
#[cfg_attr(target_arch = "wasm32", derive(Tsify))]
#[cfg_attr(target_arch = "wasm32", tsify(into_wasm_abi, from_wasm_abi))]
pub struct ResourceResponse {
    pub mem_total: u64,
    pub mem_used: u64,
    pub mem_allocated: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub swap_allocated: u64,
    pub cpu_used: f32,
    pub cpu_count: u64,
    pub disks: Vec<ResourceDisk>,
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Hash)]
#[cfg_attr(target_arch = "wasm32", derive(Tsify))]
#[cfg_attr(target_arch = "wasm32", tsify(into_wasm_abi, from_wasm_abi))]
pub struct ResourceDisk {
    pub available: u64,
    pub total: u64,
    pub mountpoint: String,
    pub readonly: bool,
    pub filesystem: String,
}

#[cfg(target_arch = "wasm32")]
impl Into<wasm_bindgen::JsValue> for ResourceResponse {
    fn into(self) -> wasm_bindgen::JsValue {
        self.into_js().unwrap().obj
    }
}
