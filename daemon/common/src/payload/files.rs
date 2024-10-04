use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct FileResponse {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub size: u64,
    pub path: Option<String>,
}
