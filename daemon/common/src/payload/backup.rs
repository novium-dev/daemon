use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct BackupResponseEntry {
    pub id: String,
    pub storage: String,
    pub time: String,
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
