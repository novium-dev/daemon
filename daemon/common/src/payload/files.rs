use serde::{Deserialize, Serialize};

#[cfg(target_arch = "wasm32")]
use super::*;

#[derive(Serialize, Deserialize)]
#[cfg_attr(target_arch = "wasm32", derive(Tsify))]
#[cfg_attr(target_arch = "wasm32", tsify(into_wasm_abi, from_wasm_abi))]
pub struct FileResponse {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub size: u64,
    pub path: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct PathUserQuery {
    pub path: String,
    pub verify: String,
    pub timestamp: String,
}

#[derive(Serialize, Deserialize)]
pub struct PathQuery {
    pub path: String,
}

#[derive(Serialize, Deserialize)]
pub struct TwoPathQuery {
    pub from: String,
    pub to: String,
}

#[derive(Serialize, Deserialize)]
pub struct PathDirectoryQuery {
    pub path: String,
    pub recursive: bool,
}

#[derive(Serialize, Deserialize)]
pub struct PathSymlinkQuery {
    pub path: String,
    pub content: String,
}
