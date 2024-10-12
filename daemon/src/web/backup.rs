use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use daemon_common::payload::TimeIdResponse;

use crate::storage::StorageIdentifier;
use crate::{storage, DaemonState};

pub(crate) async fn server_create_backup(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<Json<TimeIdResponse>, StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    storage::backup::create_backup(&state, storage)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)
        .map(|id| Json(TimeIdResponse { id }))
}

pub(crate) async fn server_download_backup(
    State(state): State<DaemonState>,
    Path((server_id, backup_id)): Path<(u32, i64)>,
) -> Result<Body, StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    storage::backup::backup_to_stream(&state, storage, backup_id)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)
}

#[cfg(feature = "user")]
pub(crate) async fn server_user_download_backup(
    State(state): State<DaemonState>,
    Path((server_id, backup_id)): Path<(u32, i64)>,
    axum::extract::Query(crate::web::user::GenericUserQuery { timestamp, verify }): axum::extract::Query<crate::web::user::GenericUserQuery>,
) -> Result<Body, StatusCode> {
    crate::web::user::authenticate_user(
        "backup",
        server_id,
        &state,
        &timestamp,
        &verify,
        Some(format!("-{}", backup_id)),
    )
    .await?;
    let storage = StorageIdentifier::Primary(server_id);
    let body = storage::backup::backup_to_stream(&state, storage, backup_id)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(body)
}

pub(crate) async fn server_backup_realtime(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<Body, StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    storage::backup::stream_realtime(&state, storage)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)
}

#[cfg(feature = "user")]
pub(crate) async fn server_user_backup_realtime(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    axum::extract::Query(crate::web::user::GenericUserQuery { timestamp, verify }): axum::extract::Query<crate::web::user::GenericUserQuery>,
) -> Result<Body, StatusCode> {
    crate::web::user::authenticate_user("backup_rt", server_id, &state, &timestamp, &verify, None)
        .await?;
    let storage = StorageIdentifier::Primary(server_id);
    storage::backup::stream_realtime(&state, storage)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)
}

pub(crate) async fn server_delete_backup(
    State(state): State<DaemonState>,
    Path((server_id, backup_id)): Path<(u32, i64)>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    storage::backup::delete_backup(&state, storage, backup_id)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)
}

pub(crate) async fn server_list_backups(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<Json<Vec<i64>>, StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    Ok(Json(storage::backup::list_backups(&state, storage).await?))
}

pub(crate) async fn server_restore_backup(
    State(state): State<DaemonState>,
    Path((server_id, backup_id)): Path<(u32, i64)>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    storage::backup::restore_backup(&state, storage, backup_id).await
}
