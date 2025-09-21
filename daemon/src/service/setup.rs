// SPDX-License-Identifier: AGPL-3.0-only
use crate::{Daemon, DaemonExtra};
use async_trait::async_trait;
use daemon_common::service::setup::SetupService;
use daemon_common::service::setup::setup_service_error::ConnectionTestError;

#[async_trait]
impl<T: DaemonExtra> SetupService for Daemon<T> {
    async fn connection_test(&self) -> Result<(), ConnectionTestError> {
        Ok(())
    }
}
