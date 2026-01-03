// SPDX-License-Identifier: AGPL-3.0-only
use crate::schedule::worker::task;
use crate::{Daemon, DaemonExtra};
use chrono_tz::Tz;
use core::str::FromStr as _;
use cron::Schedule;
use daemon_common::service::schedule::ScheduleRunPolicy;
use sqlx::types::JsonValue;
use sqlx::{query, query_scalar};
use std::sync::Arc;
use tracing::error;

pub mod worker;

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub enum ScheduleStepResult {
    Continue,
    Interrupt,
}

pub async fn init(state: &Daemon<impl DaemonExtra>) -> Result<(), sqlx::Error> {
    let mut conn = state.conn.acquire().await?;

    let mut set = state.schedule_task.lock().await;
    let mut schedules = state.schedules.lock().await;
    'sched: for schedule in query!("SELECT * FROM `schedules`")
        .fetch_all(&mut *conn)
        .await?
    {
        let policy = match schedule.policy.as_str() {
            "always" => ScheduleRunPolicy::Always,
            "running" => ScheduleRunPolicy::Running,
            "stopped" => ScheduleRunPolicy::Stopped,
            _ => continue,
        };
        let server = schedule.server;
        let cron = match Schedule::from_str(&schedule.schedule) {
            Ok(schedule) => schedule,
            Err(err) => {
                error!(
                    "Unable to parse cron schedule {:?} in {}: {:?}",
                    schedule.schedule, schedule.id, err
                );
                continue;
            }
        };
        let timezone: Tz = match schedule.timezone.parse() {
            Ok(tz) => tz,
            Err(_err) => {
                error!(
                    "Unable to find timezone in schedule {} (IANA TIMEZONE DB VERSION {}): {:?}",
                    schedule.id,
                    chrono_tz::IANA_TZDB_VERSION,
                    schedule.timezone
                );
                continue;
            }
        };

        let steps_raw = query_scalar!("SELECT `value` AS `value: JsonValue` FROM `schedule_steps` WHERE `schedule` = ? ORDER BY `index` ASC", schedule.id)
            .fetch_all(&mut *conn)
            .await?;
        let mut steps = Vec::with_capacity(steps_raw.len());
        for step in steps_raw {
            let step = match serde_json::from_value(step) {
                Ok(step) => step,
                Err(err) => {
                    error!(
                        "Unable to parse schedule step in {}: {:?}",
                        schedule.id, err
                    );
                    continue 'sched;
                }
            };
            steps.push(step);
        }

        let server_data = Arc::clone(state.servers.lock().await.entry(server).or_default());

        let handle = set.spawn(task(
            server,
            server_data,
            state.clone(),
            policy,
            steps,
            cron,
            timezone,
        ));
        schedules
            .entry(server)
            .or_default()
            .lock()
            .await
            .schedules
            .insert(schedule.id, handle);
    }
    drop(set);
    Ok(())
}
