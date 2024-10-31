use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
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
