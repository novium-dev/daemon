// SPDX-License-Identifier: AGPL-3.0-only
use crate::{Daemon, DaemonExtra, GIT_HASH, integrity};
use async_trait::async_trait;
use daemon_common::integrity::{
    IntegrityCheckInfo, IntegrityExpectation, IntegrityReport, IntegritySchema, IntegrityStatus,
};
use daemon_common::service::integrity::IntegrityService;
use daemon_common::service::integrity::integrity_service_error::{
    IntegrityChecksError, IntegrityRunError, IntegritySchemaError, IntegrityServersError,
    IntegrityStatusError,
};
use sqlx::query_scalar;

#[async_trait]
impl<T: DaemonExtra> IntegrityService for Daemon<T> {
    async fn integrity_status(&self) -> Result<IntegrityStatus, IntegrityStatusError> {
        let server_count = query_scalar!(r#"SELECT COUNT(*) AS `count: i64` FROM `servers`"#)
            .fetch_one(&*self.conn)
            .await?;

        Ok(IntegrityStatus {
            version: GIT_HASH.trim().to_owned(),
            api_hash: daemon_common::service::api_hash(self),
            uptime: self.started.elapsed().as_secs(),
            server_count: usize::try_from(server_count).unwrap_or(0usize),
        })
    }

    async fn integrity_servers(&self) -> Result<Vec<u32>, IntegrityServersError> {
        Ok(
            query_scalar!(r#"SELECT `id` AS `id: u32` FROM `servers` ORDER BY `id`"#)
                .fetch_all(&*self.conn)
                .await?,
        )
    }

    async fn integrity_checks(&self) -> Result<Vec<IntegrityCheckInfo>, IntegrityChecksError> {
        Ok(integrity::checks())
    }

    async fn integrity_run(
        &self,
        checks: Option<Vec<String>>,
        deep: bool,
        expected: Option<IntegrityExpectation>,
    ) -> Result<IntegrityReport, IntegrityRunError> {
        let ctx = self.integrity_context(expected.unwrap_or_default());

        Ok(integrity::run(&ctx, checks.as_deref(), deep).await)
    }

    async fn integrity_schema(&self) -> Result<IntegritySchema, IntegritySchemaError> {
        Ok(integrity::schema::dump(&self.conn).await?)
    }
}
