use daemon_framework::ts;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({"time": 1731160477597i64, "storage": "local"}))]
#[ts]
pub struct BackupResponseEntry {
    pub time: i64,
    pub storage: String,
    pub name: Option<String>,
    pub size: Option<u64>,
}
