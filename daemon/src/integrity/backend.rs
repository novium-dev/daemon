// SPDX-License-Identifier: AGPL-3.0-only
use super::storage::{PRIMARY_DIR, entries};
use super::{Check, CheckError, IntegrityContext};
use async_trait::async_trait;
use daemon_common::integrity::{IntegrityCategory, IntegrityFinding};
use sqlx::query_scalar;
use std::collections::HashSet;
use std::path::PathBuf;

fn report_missing(
    findings: &mut Vec<IntegrityFinding>,
    label: &str,
    expected: &HashSet<u32>,
    actual: &HashSet<u32>,
) {
    let mut missing: Vec<&u32> = expected.difference(actual).collect();
    missing.sort_unstable();

    for id in missing {
        findings.push(
            IntegrityFinding::error(format!(
                "The backend expects {label} {id} on this node, but the daemon has no record of it"
            ))
            .with_subject(id.to_string())
            .with_detail("kind", label),
        );
    }
}

pub struct BackendRecords;

#[async_trait]
impl Check for BackendRecords {
    fn id(&self) -> &'static str {
        "backend.records"
    }

    fn title(&self) -> &'static str {
        "Records the backend expects exist on this node"
    }

    fn description(&self) -> &'static str {
        "The backend hands the daemon its own view of this node when it starts a run. Anything \
         the backend has recorded but the daemon cannot find is drift the daemon alone could \
         never notice, because it has no way of knowing what it should be holding."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Database
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        let expected = &ctx.expected;
        if expected.servers.is_none() && expected.databases.is_none() && expected.storages.is_none()
        {
            return Err(CheckError::Skipped(
                "The backend did not send its view of this node".to_owned(),
            ));
        }

        let mut findings = Vec::new();

        let servers: HashSet<u32> = query_scalar!(r#"SELECT `id` AS `id: u32` FROM `servers`"#)
            .fetch_all(&*ctx.conn)
            .await?
            .into_iter()
            .collect();

        if let Some(ref wanted) = expected.servers {
            report_missing(
                &mut findings,
                "server",
                &wanted.keys().copied().collect(),
                &servers,
            );

            for (server, info) in wanted {
                let Some(ref wanted_schedules) = info.schedules else {
                    continue;
                };
                let actual: HashSet<u32> = query_scalar!(
                    r#"SELECT `id` AS `id: u32` FROM `schedules` WHERE `server` = ?"#,
                    server
                )
                .fetch_all(&*ctx.conn)
                .await?
                .into_iter()
                .collect();

                let mut missing: Vec<&u32> = wanted_schedules.difference(&actual).collect();
                missing.sort_unstable();

                for schedule in missing {
                    findings.push(
                        IntegrityFinding::error(format!(
                            "The backend expects schedule {schedule} on server {server}, but the \
                             daemon has no record of it"
                        ))
                        .with_subject(schedule.to_string())
                        .with_detail("server", server.to_string()),
                    );
                }
            }
        }

        #[cfg(feature = "databases")]
        if let Some(ref wanted) = expected.databases {
            let actual: HashSet<u32> =
                query_scalar!(r#"SELECT `id` AS `id: u32` FROM `databases`"#)
                    .fetch_all(&*ctx.conn)
                    .await?
                    .into_iter()
                    .collect();

            report_missing(&mut findings, "database", wanted, &actual);
        }

        if let Some(ref wanted) = expected.storages {
            let mut actual = HashSet::new();
            for storage in ctx.config.storage.values() {
                let root = PathBuf::from(storage.path()).join(PRIMARY_DIR);
                let Some(found) = entries(&root).await? else {
                    continue;
                };
                actual.extend(
                    found
                        .into_iter()
                        .filter(|(_name, _path, is_dir)| *is_dir)
                        .filter_map(|(name, _path, _is_dir)| name.parse::<u32>().ok()),
                );
            }

            report_missing(&mut findings, "storage", wanted, &actual);
        }

        Ok(findings)
    }
}
