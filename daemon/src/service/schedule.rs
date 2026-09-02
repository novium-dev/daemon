// SPDX-License-Identifier: AGPL-3.0-only
use crate::schedule::worker::task;
use crate::utils::server_required;
use crate::{Daemon, DaemonExtra};
use async_trait::async_trait;
use daemon_common::service::schedule::ScheduleService;
use daemon_common::service::schedule::schedule_service_error::{
    CreateScheduleError, DeleteScheduleError, ListSchedulesError, UpdateScheduleError,
};
use daemon_common::service::schedule::{SchedulePayload, ScheduleResponse, ScheduleRunPolicy};
use sqlx::types::JsonValue;
use sqlx::{Connection as _, query, query_scalar};
use tracing::error;

#[async_trait]
impl<T: DaemonExtra> ScheduleService for Daemon<T> {
    async fn list_schedules(
        &self,
        server_id: u32,
    ) -> Result<Vec<ScheduleResponse>, ListSchedulesError> {
        let _ = server_required::<ListSchedulesError>(&self.conn, &self.servers, server_id).await?;
        let mut conn = self.conn.acquire().await?;

        let rows = query!(
            "SELECT `id` AS `id: u32`, `name`, `enabled` AS `enabled: bool`, `policy`, \
             `schedule`, `timezone` FROM `schedules` WHERE `server` = ? ORDER BY `id` ASC",
            server_id
        )
        .fetch_all(&mut *conn)
        .await
        .inspect_err(|err| error!("Failed to list schedules: {err:?}"))?;

        let mut schedules = Vec::with_capacity(rows.len());
        for row in rows {
            let Some(policy) = ScheduleRunPolicy::parse(&row.policy) else {
                error!(
                    "Unable to parse run policy {:?} in schedule {}",
                    row.policy, row.id
                );
                continue;
            };

            let steps_raw = query_scalar!(
                "SELECT `value` AS `value: JsonValue` FROM `schedule_steps` WHERE `schedule` = ? \
                 ORDER BY `index` ASC",
                row.id
            )
            .fetch_all(&mut *conn)
            .await
            .inspect_err(|err| error!("Failed to list schedule steps: {err:?}"))?;

            let mut steps = Vec::with_capacity(steps_raw.len());
            for step in steps_raw {
                match serde_json::from_value(step) {
                    Ok(step) => steps.push(step),
                    Err(err) => {
                        error!("Unable to parse schedule step in {}: {err:?}", row.id);
                    }
                }
            }

            schedules.push(ScheduleResponse {
                id: row.id,
                name: row.name,
                enabled: row.enabled,
                policy,
                schedule: row.schedule,
                timezone: row.timezone,
                steps,
            });
        }

        Ok(schedules)
    }

    async fn create_schedule(
        &self,
        server_id: u32,
        payload: SchedulePayload,
    ) -> Result<u32, CreateScheduleError> {
        let (_, server_data) =
            server_required::<CreateScheduleError>(&self.conn, &self.servers, server_id).await?;
        let mut conn = self.conn.acquire().await?;

        let name = payload.name.clone();
        let enabled = payload.enabled;
        let policy = payload.policy;
        let steps = payload.steps.clone();
        let timezone = payload.timezone;
        let cron = payload.schedule.clone();

        let daemon = self.clone();
        tokio::task::spawn(async move {
            let id = conn.transaction(move |tx| Box::pin(async move {
                let id = query!("INSERT INTO `schedules` (`server`, `name`, `enabled`, `policy`, `schedule`, `timezone`) VALUES (?, ?, ?, ?, ?, ?)", server_id, name, enabled, policy.as_str(), cron.to_string(), timezone.to_string())
                    .execute(&mut **tx)
                    .await
                    .inspect_err(|err| error!("Failed to insert schedule: {:?}", err))?
                    .last_insert_id() as u32;

                for (index, step) in steps.into_iter().enumerate() {
                    let value = serde_json::to_value(step).expect("Failed to serialize schedule step");
                    query!("INSERT INTO `schedule_steps` (`schedule`, `index`, `value`) VALUES (?, ?, ?)", id, index as u32, value)
                        .execute(&mut **tx)
                        .await
                        .inspect_err(|err| error!("Failed to insert schedule step: {:?}", err))?;
                }

                Ok::<_, CreateScheduleError>(id)
            })).await?;

            if enabled {
                let handle = daemon.schedule_task.lock().await.spawn(task(
                    server_id,
                    server_data,
                    daemon.clone(),
                    payload.policy,
                    payload.steps,
                    payload.schedule,
                    payload.timezone,
                ));
                daemon.schedules
                    .lock()
                    .await
                    .entry(server_id)
                    .or_default()
                    .lock()
                    .await
                    .schedules
                    .insert(id, handle);
            }
            Ok(id)
        }).await.expect("Schedule create task panicked")
    }

    async fn update_schedule(
        &self,
        server_id: u32,
        schedule_id: u32,
        payload: SchedulePayload,
    ) -> Result<(), UpdateScheduleError> {
        let (_, server_data) =
            server_required::<UpdateScheduleError>(&self.conn, &self.servers, server_id).await?;
        let mut conn = self.conn.acquire().await?;

        let name = payload.name.clone();
        let enabled = payload.enabled;
        let policy = payload.policy;
        let steps = payload.steps.clone();
        let timezone = payload.timezone;
        let cron = payload.schedule.clone();

        let daemon = self.clone();
        tokio::task::spawn(async move {
            conn.transaction(move |tx| Box::pin(async move {
                let Some(_) = query_scalar!("SELECT `id` FROM `schedules` WHERE `id` = ? AND `server` = ?", schedule_id, server_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .inspect_err(|err| error!("Failed to select schedule: {:?}", err))? else {
                    return Err(UpdateScheduleError::ScheduleNotFound);
                };

                query!("UPDATE `schedules` SET `name` = ?, `enabled` = ?, `policy` = ?, `schedule` = ?, `timezone` = ? WHERE `id` = ? AND `server` = ?", name, enabled, policy.as_str(), cron.to_string(), timezone.to_string(), schedule_id, server_id)
                    .execute(&mut **tx)
                    .await
                    .inspect_err(|err| error!("Failed to update schedule: {err:?}"))?;

                query!("DELETE FROM `schedule_steps` WHERE `schedule` = ?", schedule_id)
                    .execute(&mut **tx)
                    .await
                    .inspect_err(|err| error!("Failed to delete schedule steps: {err:?}"))?;

                for (index, step) in steps.into_iter().enumerate() {
                    let value = serde_json::to_value(step).expect("Failed to serialize schedule step");
                    query!("INSERT INTO `schedule_steps` (`schedule`, `index`, `value`) VALUES (?, ?, ?)", schedule_id, index as u32, value)
                        .execute(&mut **tx)
                        .await
                        .inspect_err(|err| error!("Failed to insert schedule step (update): {err:?}"))?;
                }

                Ok::<_, UpdateScheduleError>(())
            })).await?;

            let mut schedules = daemon.schedules.lock().await;
            let mut server_schedules = schedules.entry(server_id).or_default().lock().await;

            if let Some(handle) = server_schedules.schedules.remove(&schedule_id) {
                handle.abort();
            }

            if enabled {
                let handle = daemon.schedule_task.lock().await.spawn(task(
                    server_id,
                    server_data,
                    daemon.clone(),
                    payload.policy,
                    payload.steps,
                    payload.schedule,
                    payload.timezone,
                ));

                server_schedules.schedules.insert(schedule_id, handle);
            }
            drop(server_schedules);
            drop(schedules);

            Ok(())
        }).await.expect("Schedule update task panicked")
    }

    async fn delete_schedule(
        &self,
        server_id: u32,
        schedule_id: u32,
    ) -> Result<(), DeleteScheduleError> {
        let _ =
            server_required::<DeleteScheduleError>(&self.conn, &self.servers, server_id).await?;
        let mut conn = self.conn.acquire().await?;
        let daemon = self.clone();
        tokio::task::spawn(async move {
            query!(
                "DELETE FROM `schedules` WHERE `id` = ? AND `server` = ?",
                schedule_id,
                server_id
            )
            .execute(&mut *conn)
            .await
            .inspect_err(|err| error!("Failed to delete schedule: {:?}", err))?;

            let mut schedules = daemon.schedules.lock().await;
            if let Some(schedules) = schedules.get_mut(&server_id) {
                let schedule = schedules.lock().await.schedules.remove(&schedule_id);
                if let Some(sender) = schedule {
                    sender.abort();
                }
            }
            drop(schedules);

            Ok(())
        })
        .await
        .expect("Delete schedule task panicked")
    }
}
