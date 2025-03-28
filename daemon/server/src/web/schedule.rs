use crate::DaemonState;
use crate::schedule::worker::task;
use crate::utils::server;
use crate::web::sql_error_to_http;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use daemon_common::payload::IdResponse;
use daemon_common::payload::schedule::{SchedulePayload, ScheduleRunPolicy};
use sqlx::{Connection, query, query_scalar};
use std::ops::Deref;
use tracing::error;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub(crate) fn config() -> OpenApiRouter<DaemonState> {
    OpenApiRouter::new()
        .routes(routes!(create_schedule))
        .routes(routes!(update_schedule))
        .routes(routes!(delete_schedule))
}

#[utoipa::path(put,
    path = "/schedule/{server_id}",
    tag = "schedule",
    description = "Creates a new schedule",
    responses(
        (status = 200, description = "Schedule was created successfully", body = IdResponse, content_type = "application/json"),
        (status = 404, description = "Server does not exist"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid"),
        (status = 400, description = "Invalid schedule")
    ),
    params(
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
async fn create_schedule(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Json(payload): Json<SchedulePayload>,
) -> Result<Json<IdResponse>, StatusCode> {
    let Some(_) = server(state.conn.deref(), &state.servers, server_id)
        .await
        .map_err(sql_error_to_http)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    let mut conn = state.conn.acquire().await.map_err(sql_error_to_http)?;

    let policy = payload.policy;
    let steps = payload.steps.clone();
    let timezone = payload.timezone;
    let cron = payload.schedule.clone();

    let id = conn.transaction(move |tx| Box::pin(async move {
        let id = query!("INSERT INTO `schedules` (`server`, `policy`, `schedule`, `timezone`) VALUES (?, ?, ?, ?)", server_id, match policy {
            ScheduleRunPolicy::Always => "always",
            ScheduleRunPolicy::Running => "running",
            ScheduleRunPolicy::Stopped => "stopped",
        }, cron.to_string(), timezone.to_string())
            .execute(&mut **tx)
            .await
            .inspect_err(|err| error!("Failed to insert schedule: {:?}", err))?
            .last_insert_id() as u32;

        for (index, step) in steps.into_iter().enumerate() {
            let value = serde_json::to_value(step).unwrap();
            query!("INSERT INTO `schedule_steps` (`schedule`, `index`, `value`) VALUES (?, ?, ?)", id, index as u32, value)
                .execute(&mut **tx)
                .await
                .inspect_err(|err| error!("Failed to insert schedule step: {:?}", err))?;
        }

        Ok(id)
    })).await
        .map_err(sql_error_to_http)?;

    let (send, recv) = tokio::sync::oneshot::channel();
    drop(tokio::task::spawn(task(
        server_id,
        state.clone(),
        payload.policy,
        payload.steps,
        recv,
        payload.schedule,
        payload.timezone,
    )));
    state
        .schedules
        .lock()
        .await
        .entry(server_id)
        .or_default()
        .lock()
        .await
        .schedules
        .insert(id, send);

    Ok(Json(IdResponse { id }))
}

#[utoipa::path(patch,
    path = "/schedule/{server_id}/{schedule_id}",
    tag = "schedule",
    description = "Updates a schedule",
    responses(
        (status = 200, description = "Schedule was updated successfully"),
        (status = 404, description = "Server or Schedule does not exist"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid"),
        (status = 400, description = "Invalid schedule")
    ),
    params(
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
async fn update_schedule(
    State(state): State<DaemonState>,
    Path((server_id, schedule_id)): Path<(u32, u32)>,
    Json(payload): Json<SchedulePayload>,
) -> Result<(), StatusCode> {
    let Some(_) = server(state.conn.deref(), &state.servers, server_id)
        .await
        .map_err(sql_error_to_http)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    let mut conn = state.conn.acquire().await.map_err(sql_error_to_http)?;

    let policy = payload.policy;
    let steps = payload.steps.clone();
    let timezone = payload.timezone;
    let cron = payload.schedule.clone();

    conn.transaction(move |tx| Box::pin(async move {
        let Some(_) = query_scalar!("SELECT `id` FROM `schedules` WHERE `id` = ? AND `server` = ?", schedule_id, server_id)
            .fetch_optional(&mut **tx)
            .await
            .inspect_err(|err| error!("Failed to select schedule: {:?}", err))? else {
            return Err(sqlx::Error::RowNotFound);
        };

        query!("UPDATE `schedules` SET `policy` = ?, `schedule` = ?, `timezone` = ? WHERE `id` = ? AND `server` = ?", match policy {
            ScheduleRunPolicy::Always => "always",
            ScheduleRunPolicy::Running => "running",
            ScheduleRunPolicy::Stopped => "stopped",
        }, cron.to_string(), timezone.to_string(), schedule_id, server_id)
            .execute(&mut **tx)
            .await
            .inspect_err(|err| error!("Failed to update schedule: {:?}", err))?;

        query!("DELETE FROM `schedule_steps` WHERE `schedule` = ?", schedule_id)
            .execute(&mut **tx)
            .await
            .inspect_err(|err| error!("Failed to delete schedule steps: {:?}", err))?;

        for (index, step) in steps.into_iter().enumerate() {
            let value = serde_json::to_value(step).unwrap();
            query!("INSERT INTO `schedule_steps` (`schedule`, `index`, `value`) VALUES (?, ?, ?)", schedule_id, index as u32, value)
                .execute(&mut **tx)
                .await
                .inspect_err(|err| error!("Failed to insert schedule step (update): {:?}", err))?;
        }

        Ok(())
    })).await
        .map_err(|err| {
            if let sqlx::Error::RowNotFound = err {
                StatusCode::NOT_FOUND
            } else {
                sql_error_to_http(err)
            }
        })?;

    let mut schedules = state.schedules.lock().await;
    let mut schedules = schedules.entry(server_id).or_default().lock().await;

    if let Some(sender) = schedules.schedules.remove(&schedule_id) {
        let _ = sender.send(());
    }

    let (send, recv) = tokio::sync::oneshot::channel();
    drop(tokio::task::spawn(task(
        server_id,
        state.clone(),
        payload.policy,
        payload.steps,
        recv,
        payload.schedule,
        payload.timezone,
    )));

    schedules.schedules.insert(schedule_id, send);

    Ok(())
}

#[utoipa::path(delete,
    path = "/schedule/{server_id}/{schedule_id}",
    tag = "schedule",
    description = "Deletes a schedule",
    responses(
        (status = 200, description = "Schedule was deleted successfully or doesn't exist"),
        (status = 404, description = "Server does not exist"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
async fn delete_schedule(
    State(state): State<DaemonState>,
    Path((server_id, schedule_id)): Path<(u32, u32)>,
) -> Result<(), StatusCode> {
    let Some(_) = server(state.conn.deref(), &state.servers, server_id)
        .await
        .map_err(sql_error_to_http)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    let mut conn = state.conn.acquire().await.map_err(sql_error_to_http)?;
    query!(
        "DELETE FROM `schedules` WHERE `id` = ? AND `server` = ?",
        schedule_id,
        server_id
    )
    .execute(&mut *conn)
    .await
    .inspect_err(|err| error!("Failed to delete schedule: {:?}", err))
    .map_err(sql_error_to_http)?;

    if let Some(schedules) = state.schedules.lock().await.get_mut(&server_id) {
        if let Some(sender) = schedules.lock().await.schedules.remove(&schedule_id) {
            let _ = sender.send(());
        }
    }

    Ok(())
}
