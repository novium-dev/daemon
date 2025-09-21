// SPDX-License-Identifier: AGPL-3.0-only
use super::*;
use zelus::service;

#[service(tag = "setup")]
pub trait SetupService {
    /// Tests the connection
    #[route("/setup", method = GET)]
    #[error({ AwaitingSetup "This error is expected while in the setup wizard" FORBIDDEN })]
    async fn connection_test(&self) -> Result<(), _>;
}
