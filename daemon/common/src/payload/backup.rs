use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({"time": 1731160477597i64, "storage": "local"}))]
pub struct BackupResponseEntry {
    pub time: i64,
    pub storage: String,
    pub name: Option<String>,
    pub size: Option<u64>,
}

#[derive(Serialize, Deserialize)]
pub struct CompressionQuery {
    pub compression: String,
}

#[derive(Serialize, Deserialize)]
pub struct UserCompressionQuery {
    pub timestamp: String,
    pub verify: String,
    pub compression: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({"name": null}))]
pub struct BackupUpdatePayload {
    pub name: Option<String>,
}
