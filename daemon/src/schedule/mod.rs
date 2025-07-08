/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
use crate::schedule::worker::task;
use crate::{Daemon, DaemonExtra};
use chrono_tz::Tz;
use core::str::FromStr as _;
use cron::Schedule;
use daemon_common::service::schedule::ScheduleRunPolicy;
use sqlx::types::JsonValue;
use sqlx::{query, query_scalar};
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

        let handle = set.spawn(task(server, state.clone(), policy, steps, cron, timezone));
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
