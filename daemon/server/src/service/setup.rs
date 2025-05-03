use crate::Daemon;
use async_trait::async_trait;
use daemon_common::service::setup::SetupService;
use daemon_common::service::setup::error::ConnectionTestError;

#[async_trait]
impl SetupService for Daemon {
    async fn connection_test(&self) -> Result<(), ConnectionTestError> {
        Ok(())
    }
}
