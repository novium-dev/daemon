// SPDX-License-Identifier: AGPL-3.0-only
use crate::node::BackendAuthServerPermissions;
use serde::{Deserialize, Serialize};

pub type DaemonBackendAuthResponse = Vec<DaemonBackendAuthServer>;
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DaemonBackendAuthServer {
    pub id: u32,
    pub name: String,
    pub permissions: BackendAuthServerPermissions,
}
