// SPDX-License-Identifier: AGPL-3.0-only
mod backup;
mod config;
mod database;
mod docker;
pub mod schema;
mod storage;

use crate::config::DaemonConfig;
use crate::{Daemon, DaemonExtra};
use async_trait::async_trait;
use bollard::Docker;
use core::time::Duration;
use daemon_common::integrity::{
    IntegrityCategory, IntegrityCheckInfo, IntegrityCheckResult, IntegrityFinding, IntegrityReport,
    IntegritySeverity,
};
use sqlx::{MySql, Pool};
use std::io;
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, warn};

pub const CHECK_TIMEOUT: Duration = Duration::from_secs(600);

pub struct IntegrityContext {
    pub config: Arc<DaemonConfig>,
    pub conn: Arc<Pool<MySql>>,
    pub docker: Arc<Docker>,
}

impl<T: DaemonExtra> Daemon<T> {
    pub(crate) fn integrity_context(&self) -> IntegrityContext {
        IntegrityContext {
            config: Arc::clone(&self.config),
            conn: Arc::clone(&self.conn),
            docker: Arc::clone(&self.docker),
        }
    }
}

pub enum CheckError {
    Skipped(String),
    Failed(String),
}

impl From<sqlx::Error> for CheckError {
    fn from(err: sqlx::Error) -> Self {
        Self::Failed(format!("database error: {err}"))
    }
}

impl From<io::Error> for CheckError {
    fn from(err: io::Error) -> Self {
        Self::Failed(format!("io error: {err}"))
    }
}

impl From<bollard::errors::Error> for CheckError {
    fn from(err: bollard::errors::Error) -> Self {
        Self::Failed(format!("docker error: {err}"))
    }
}

#[async_trait]
pub trait Check: Send + Sync {
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn category(&self) -> IntegrityCategory;

    fn deep(&self) -> bool {
        false
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError>;
}

fn catalogue() -> Vec<Box<dyn Check>> {
    vec![
        Box::new(docker::DockerReachable),
        Box::new(docker::DockerContainers),
        Box::new(docker::DockerNetworks),
        Box::new(database::MariadbDatabases),
        Box::new(database::MariadbUsers),
        Box::new(config::ConfigPaths),
        Box::new(storage::StorageNaming),
        Box::new(storage::StorageSubvolume),
        Box::new(backup::BackupNaming),
        Box::new(backup::BackupCompression),
        Box::new(backup::BackupSubvolume),
        Box::new(backup::BackupTrailer),
        Box::new(schema::SchemaDump),
    ]
}

#[must_use]
pub fn checks() -> Vec<IntegrityCheckInfo> {
    catalogue()
        .iter()
        .map(|check| IntegrityCheckInfo {
            id: check.id().to_owned(),
            title: check.title().to_owned(),
            description: check.description().to_owned(),
            category: check.category(),
            deep: check.deep(),
        })
        .collect()
}

pub async fn run(
    ctx: &IntegrityContext,
    selected: Option<&[String]>,
    deep: bool,
) -> IntegrityReport {
    let started = Instant::now();
    let mut results = Vec::new();

    for check in catalogue() {
        let wanted = selected.is_none_or(|ids| ids.iter().any(|id| id == check.id()));
        if !wanted || (check.deep() && !deep) {
            continue;
        }
        results.push(run_one(ctx, check.as_ref()).await);
    }

    IntegrityReport::new(
        crate::GIT_HASH.trim().to_owned(),
        elapsed_ms(started),
        results,
    )
}

async fn run_one(ctx: &IntegrityContext, check: &dyn Check) -> IntegrityCheckResult {
    let started = Instant::now();
    let outcome = tokio::time::timeout(CHECK_TIMEOUT, check.run(ctx)).await;

    let (severity, findings, skipped) = match outcome {
        Ok(Ok(findings)) => (IntegrityCheckResult::severity_of(&findings), findings, None),
        Ok(Err(CheckError::Skipped(reason))) => (IntegritySeverity::Ok, Vec::new(), Some(reason)),
        Ok(Err(CheckError::Failed(reason))) => {
            error!("Integrity check {} failed: {reason}", check.id());
            (
                IntegritySeverity::Error,
                vec![IntegrityFinding::error(format!(
                    "The check could not be completed: {reason}"
                ))],
                None,
            )
        }
        Err(_elapsed) => {
            warn!("Integrity check {} timed out", check.id());
            (
                IntegritySeverity::Error,
                vec![IntegrityFinding::error(
                    "The check timed out before it could finish".to_owned(),
                )],
                None,
            )
        }
    };

    IntegrityCheckResult {
        id: check.id().to_owned(),
        title: check.title().to_owned(),
        description: check.description().to_owned(),
        category: check.category(),
        deep: check.deep(),
        severity,
        duration_ms: elapsed_ms(started),
        findings,
        skipped,
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
