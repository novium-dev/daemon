use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({"name": "eula.txt", "uid": 0, "gid": 0, "mode": 0o644, "size": 64, "path": null}))]
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

#[deprecated]
#[derive(Serialize, Deserialize)]
pub struct PathUserQuery {
    pub path: String,
    pub verify: String,
    pub timestamp: String,
}

#[deprecated]
#[derive(Serialize, Deserialize)]
pub struct PathQuery {
    pub path: String,
}

#[deprecated]
#[derive(Serialize, Deserialize)]
pub struct TwoPathQuery {
    pub from: String,
    pub to: String,
}

#[deprecated]
#[derive(Serialize, Deserialize)]
pub struct PathDirectoryQuery {
    pub path: String,
    pub recursive: bool,
}

#[deprecated]
#[derive(Serialize, Deserialize)]
pub struct PathSymlinkQuery {
    pub path: String,
    pub content: String,
}

#[deprecated]
#[derive(Serialize, Deserialize)]
pub struct PathUrlQuery {
    pub path: String,
    pub url: String,
}

#[deprecated]
#[derive(Serialize, Deserialize)]
pub struct PathDestinationQuery {
    pub path: String,
    pub dest: String,
}

#[deprecated]
#[derive(Serialize, Deserialize)]
pub struct PathCompressionSourceQuery {
    pub path: String,
    pub src: String,
    pub compression: String,
}
