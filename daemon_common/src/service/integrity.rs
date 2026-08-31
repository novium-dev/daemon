// SPDX-License-Identifier: AGPL-3.0-only
use super::*;
use crate::integrity::{
    IntegrityCheckInfo, IntegrityExpectation, IntegrityReport, IntegritySchema, IntegrityStatus,
};
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

    /// Lists the ids of every server in the daemon database, so the backend can
    /// diff them against the servers it believes this node runs.
    #[route("/servers", method = GET)]
    #[error(sql::failure <- [sql:*])]
    async fn integrity_servers(&self) -> Result<Vec<u32>, _>;

    /// Lists every integrity check this daemon can run.
    #[route("/checks", method = GET)]
    #[error()]
    async fn integrity_checks(&self) -> Result<Vec<IntegrityCheckInfo>, _>;

    /// Runs the selected integrity checks and returns their report.
    ///
    /// Passing no selection runs every check that is not marked as deep, unless
    /// `deep` is set. `expected` carries the backend's own view of this node so
    /// the daemon can report records the backend knows about but it does not.
    #[route("/run", method = PUT)]
    #[error(integrity::unavailable)]
    async fn integrity_run(
        &self,
        checks: Option<Vec<String>>,
        deep: bool,
        expected: Option<IntegrityExpectation>,
    ) -> Result<IntegrityReport, _>;

    /// Dumps the `CREATE TABLE` statement of every table in the daemon database.
    #[route("/schema", method = GET)]
    #[error(sql::failure <- [sql:*])]
    async fn integrity_schema(&self) -> Result<IntegritySchema, _>;
}
