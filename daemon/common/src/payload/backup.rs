use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({"time": 1731160477597i64, "storage": "local"}))]
pub struct BackupResponseEntry {
    pub time: i64,
    pub storage: String,
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
