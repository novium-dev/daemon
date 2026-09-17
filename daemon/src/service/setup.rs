// SPDX-License-Identifier: AGPL-3.0-only
use crate::{Daemon, DaemonExtra};
use async_trait::async_trait;
use daemon_common::node::BackendNodeConfig;
use daemon_common::service::setup::SetupService;
use daemon_common::service::setup::setup_service_error::ConfigError;

#[async_trait]
impl<T: DaemonExtra> SetupService for Daemon<T> {
    async fn config(&self) -> Result<BackendNodeConfig, ConfigError> {
        Ok(self.config.to_node_config())
    }
}
