use daemon_framework::ts;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({"name": "eula.txt", "uid": 0, "gid": 0, "mode": 0o644, "size": 64, "path": null}))]
#[ts]
pub struct FileEntry {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub size: u64,
    pub relative_path: String, // for recursive listing like /plugins/essentials
    pub path: Option<String>,  // for symlinks
    pub archive: bool,
}
