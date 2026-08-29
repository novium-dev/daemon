// SPDX-License-Identifier: AGPL-3.0-only
use super::*;
use crate::integrity::{IntegrityCheckInfo, IntegrityReport, IntegritySchema, IntegrityStatus};
use zelus::service;

define_error!(integrity {
    unavailable ("The integrity subsystem is unavailable" INTERNAL_SERVER_ERROR),
});

#[service(path = "/integrity", tag = "integrity")]
pub trait IntegrityService {
    /// Returns the daemon build, uptime and the number of servers it knows about.
    #[route("/status", method = GET)]
    #[error(sql::failure <- [sql:*])]
    async fn integrity_status(&self) -> Result<IntegrityStatus, _>;

    /// Lists every integrity check this daemon can run.
    #[route("/checks", method = GET)]
    #[error()]
    async fn integrity_checks(&self) -> Result<Vec<IntegrityCheckInfo>, _>;

    /// Runs the selected integrity checks and returns their report.
    ///
    /// Passing no selection runs every check that is not marked as deep, unless
    /// `deep` is set.
    #[route("/run", method = PUT)]
    #[error(integrity::unavailable)]
    async fn integrity_run(
        &self,
        checks: Option<Vec<String>>,
        deep: bool,
    ) -> Result<IntegrityReport, _>;

    /// Dumps the `CREATE TABLE` statement of every table in the daemon database.
    #[route("/schema", method = GET)]
    #[error(sql::failure <- [sql:*])]
    async fn integrity_schema(&self) -> Result<IntegritySchema, _>;
}
