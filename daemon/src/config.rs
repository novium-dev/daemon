use ed25519_dalek::VerifyingKey;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct DaemonConfig {
    pub(crate) identifier: String,
    pub(crate) token: String,
    pub(crate) websocket_key: VerifyingKey,
    pub(crate) websocket_allow_duration: u128,
    pub(crate) origin: String,
    pub(crate) storage_path: String,
    pub(crate) btrfs: bool,
    pub(crate) bind: String,
    pub(crate) ssh_bind: String,
    pub(crate) database: String,
}
