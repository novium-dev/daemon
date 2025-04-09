use crate::Daemon;
use crate::schedule::worker::task;
use chrono_tz::Tz;
use cron::Schedule;
use daemon_common::payload::schedule::ScheduleRunPolicy;
use sqlx::types::JsonValue;
use sqlx::{query, query_scalar};
use std::str::FromStr;
use tracing::error;

pub(crate) mod worker;

#[derive(Clone, Copy, Hash)]
pub(crate) enum ScheduleStepResult {
    Continue,
    Interrupt,
}

pub(crate) async fn init(state: &Daemon) -> Result<(), sqlx::Error> {
    let mut conn = state.conn.acquire().await?;
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
        let (send, recv) = tokio::sync::oneshot::channel();
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

        drop(tokio::task::spawn(task(
            server,
            state.clone(),
            policy,
            steps,
            recv,
            cron,
            timezone,
        )));
        schedules
            .entry(server)
            .or_default()
            .lock()
            .await
            .schedules
            .insert(schedule.id, send);
    }
    Ok(())
}
