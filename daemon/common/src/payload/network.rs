use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkPayload {
    pub ipv4: u16,
    pub ipv6: u16,
}
