// SPDX-License-Identifier: AGPL-3.0-only
use super::*;
use crate::node::BackendNodeConfig;
use zelus::service;

#[service(tag = "setup")]
pub trait SetupService {
    /// Returns the daemon's current config (backend-driven update)
    #[route("/setup/config", method = GET)]
    #[error((AwaitingSetup, "This error is expected while in the setup wizard", FORBIDDEN))]
    async fn config(&self) -> Result<BackendNodeConfig, _>;
}
