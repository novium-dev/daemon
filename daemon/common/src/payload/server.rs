use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize)]
pub struct ServerPayload {
    pub resource: Option<ServerResourcePayload>,
    pub build: Option<ServerBuildPayload>,
    pub ports: Option<Vec<ServerPortEntryPayload>>,
}

#[derive(Serialize, Deserialize)]
pub struct ServerResourcePayload {
    pub cpu: Option<f32>,
    pub ram: Option<u32>,
    pub swap: Option<u32>,
    pub disk: Option<u64>,
}

#[derive(Serialize, Deserialize)]
pub struct ServerBuildPayload {
    pub identifier: String,
    pub dockerfile: String,
    pub build_args: HashMap<String, String>,

    pub cap_add: Vec<String>,
    pub cap_drop: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct ServerPortEntryPayload {
    pub internal_port: u16,
    pub external_port: u16,
    pub external_ip: String,
    pub protocol: String,
}
