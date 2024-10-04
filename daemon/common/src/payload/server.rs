use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use super::*;

#[derive(Serialize, Deserialize)]
pub struct ServerPayload {
    pub resource: Option<ServerResourcePayload>,
    pub build: Option<ServerBuildPayload>,
    pub ports: Option<Vec<ServerPortEntryPayload>>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(target_arch = "wasm32", derive(Tsify))]
#[cfg_attr(target_arch = "wasm32", tsify(into_wasm_abi, from_wasm_abi))]
pub struct ServerResourcePayload {
    pub cpu: Option<f32>,
    pub ram: Option<u32>,
    pub swap: Option<u32>,
    pub disk: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(target_arch = "wasm32", derive(Tsify))]
#[cfg_attr(target_arch = "wasm32", tsify(into_wasm_abi, from_wasm_abi))]
pub struct ServerBuildPayload {
    pub identifier: String,
    pub dockerfile: String,
    pub build_args: HashMap<String, String>,

    pub cap_add: Vec<String>,
    pub cap_drop: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(target_arch = "wasm32", derive(Tsify))]
#[cfg_attr(target_arch = "wasm32", tsify(into_wasm_abi, from_wasm_abi))]
pub struct ServerPortEntryPayload {
    pub internal_port: u16,
    pub external_port: u16,
    pub external_ip: String,
    pub protocol: String,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(target_arch = "wasm32", derive(Tsify))]
#[cfg_attr(target_arch = "wasm32", tsify(into_wasm_abi, from_wasm_abi))]
#[serde(transparent)]
pub struct ServerPortsPayload(pub Vec<ServerPortEntryPayload>);

#[cfg(target_arch = "wasm32")]
impl Into<Vec<ServerPortEntryPayload>> for ServerPortsPayload {
    fn into(self) -> Vec<ServerPortEntryPayload> {
        self.0
    }
}
