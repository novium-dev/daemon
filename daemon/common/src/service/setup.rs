use super::*;
use daemon_framework::service;

#[service(tag = "setup")]
pub trait SetupService {
    #[route("/setup", method = GET)]
    #[error({ AwaitingSetup "This error is expected while in the setup wizard" FORBIDDEN })]
    #[doc = "Tests the connection"]
    async fn connection_test(&self) -> Result<(), _>;
}
