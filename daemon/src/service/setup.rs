use crate::{Daemon, DaemonExtra};
use async_trait::async_trait;
use daemon_common::service::setup::SetupService;
use daemon_common::service::setup::error::ConnectionTestError;

#[async_trait]
impl<T: DaemonExtra> SetupService for Daemon<T> {
    async fn connection_test(&self) -> Result<(), ConnectionTestError> {
        Ok(())
    }
}
