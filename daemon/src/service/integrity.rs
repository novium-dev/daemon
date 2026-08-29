// SPDX-License-Identifier: AGPL-3.0-only
use crate::{Daemon, DaemonExtra, GIT_HASH, integrity};
use async_trait::async_trait;
use daemon_common::integrity::{
    IntegrityCheckInfo, IntegrityReport, IntegritySchema, IntegrityStatus,
};
use daemon_common::service::integrity::IntegrityService;
use daemon_common::service::integrity::integrity_service_error::{
    IntegrityChecksError, IntegrityRunError, IntegritySchemaError, IntegrityStatusError,
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
            uptime: self.started.elapsed().as_secs(),
            server_count: usize::try_from(server_count).unwrap_or(0usize),
        })
    }

    async fn integrity_checks(&self) -> Result<Vec<IntegrityCheckInfo>, IntegrityChecksError> {
        Ok(integrity::checks())
    }

    async fn integrity_run(
        &self,
        checks: Option<Vec<String>>,
        deep: bool,
    ) -> Result<IntegrityReport, IntegrityRunError> {
        Ok(integrity::run(&self.integrity_context(), checks.as_deref(), deep).await)
    }

    async fn integrity_schema(&self) -> Result<IntegritySchema, IntegritySchemaError> {
        Ok(integrity::schema::dump(&self.conn).await?)
    }
}
