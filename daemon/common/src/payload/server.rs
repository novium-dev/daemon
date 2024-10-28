use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ServerPayload {
    pub storage: Option<String>,
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

    pub backup_exclude: HashSet<String>,

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

#[derive(Serialize, Deserialize)]
#[serde(transparent)]
pub struct ServerPortsPayload(pub Vec<ServerPortEntryPayload>);

impl From<ServerPortsPayload> for Vec<ServerPortEntryPayload> {
    fn from(value: ServerPortsPayload) -> Self {
        value.0
    }
}

impl From<Vec<ServerPortEntryPayload>> for ServerPortsPayload {
    fn from(value: Vec<ServerPortEntryPayload>) -> Self {
        ServerPortsPayload(value)
    }
}
