use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize)]
pub(crate) struct ServerPayload {
    pub(crate) resource: Option<ServerResourcePayload>,
    pub(crate) build: Option<ServerBuildPayload>,

    pub(crate) ports: Option<Vec<ServerPortEntryPayload>>,
}

#[derive(Deserialize)]
pub(crate) struct ServerResourcePayload {
    pub(crate) cpu: Option<f32>,
    pub(crate) ram: Option<u32>,
    pub(crate) swap: Option<u32>,
    pub(crate) disk: Option<u64>,
}

#[derive(Deserialize)]
pub(crate) struct ServerBuildPayload {
    pub(crate) identifier: String,
    pub(crate) dockerfile: String,
    pub(crate) build_args: HashMap<String, String>,

    pub(crate) cap_add: Vec<String>,
    pub(crate) cap_drop: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct ServerPortEntryPayload {
    pub(crate) internal_port: u16,
    pub(crate) external_port: u16,
    pub(crate) external_ip: String,
    pub(crate) protocol: String,
}

#[derive(Serialize)]
pub(crate) struct IdResponse {
    pub(crate) id: u32,
}
