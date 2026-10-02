use async_trait::async_trait;
use daemon_common::service::group::{
    GroupService,
    group_service_error::{ConfigureGroupError, DeleteGroupError},
};
use sqlx::query;
use tracing::error;

use crate::{Daemon, DaemonExtra};

#[async_trait]
impl<T: DaemonExtra> GroupService for Daemon<T> {
    async fn configure_group(
        &self,
        group_name: String,
        cpu: Option<f32>,
        ram: Option<u32>,
        swap: Option<u32>,
    ) -> Result<(), ConfigureGroupError> {
        query!("INSERT INTO `groups` (`name`, `cpu`, `ram`, `swap`) VALUES (?, ?, ?, ?) ON DUPLICATE KEY UPDATE `cpu` = ?, `ram` = ?, `swap` = ?", group_name, cpu, ram, swap, cpu, ram, swap)
        .execute(&*self.conn)
        .await
        .inspect_err(|err| error!("Failed to configure group: {err:?}"))?;

        self.sync_cgroups::<ConfigureGroupError>().await?;

        Ok(())
    }

    async fn delete_group(&self, group_name: String) -> Result<(), DeleteGroupError> {
        query!("DELETE FROM `groups` WHERE `name` = ?", group_name)
            .execute(&*self.conn)
            .await
            .inspect_err(|err| error!("Failed to delete group: {err:?}"))?;

        self.sync_cgroups::<DeleteGroupError>().await?;

        Ok(())
    }
}
