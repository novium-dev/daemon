use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use daemon_common::payload::backup::*;
use daemon_common::payload::TimeIdResponse;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::storage::StorageIdentifier;
use crate::{storage, DaemonState};

pub(crate) fn config() -> OpenApiRouter<DaemonState> {
    OpenApiRouter::new()
        .routes(routes!(server_create_backup))
        .routes(routes!(server_create_backup_order))
        .routes(routes!(server_download_backup))
        .routes(routes!(server_backup_realtime))
        .routes(routes!(server_delete_backup))
        .routes(routes!(server_list_backups))
        .routes(routes!(server_restore_backup))
        .routes(routes!(server_update_backup))
}

#[cfg(feature = "user")]
pub(crate) fn user_config() -> OpenApiRouter<DaemonState> {
    OpenApiRouter::new()
        .routes(routes!(server_user_download_backup))
        .routes(routes!(server_user_backup_realtime))
}

#[utoipa::path(put,
    path = "/backup/{server_id}/{backup_storage}",
    tag = "backup",
    description = "Creates a new backup",
    responses(
        (status = 200, description = "Backup was created successfully", body = TimeIdResponse, content_type = "application/json"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server or backup storage does not exist"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = i64, description = "The id of the server"),
        ("backup_storage" = String, description = "The id of the storage, in which the backup will be stored"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_create_backup(
    State(state): State<DaemonState>,
    Path((server_id, backup_storage)): Path<(u32, String)>,
) -> Result<Json<TimeIdResponse>, StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    storage::backup::create_backup(&state, storage, &backup_storage)
        .await
        .map(|time| Json(TimeIdResponse { time }))
}

#[utoipa::path(put,
    path = "/backup/{server_id}",
    tag = "backup",
    description = "Creates a new backup",
    responses(
        (status = 200, description = "Backup was created successfully", body = TimeIdResponse, content_type = "application/json"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server does not exist"),
        (status = 401, description = "Your authorization token is invalid"),
        (status = 417, description = "No suitable backup storage available")
    ),
    params(
        ("server_id" = i64, description = "The id of the server"),
        ("backup_storage" = String, description = "The id of the storage, in which the backup will be stored"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_create_backup_order(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<Json<TimeIdResponse>, StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    storage::backup::create_backup_order(&state, storage)
        .await
        .map(|time| Json(TimeIdResponse { time }))
}

#[utoipa::path(get,
    path = "/backup/{server_id}/{backup_storage}/{backup_id}",
    tag = "backup",
    description = "Downloads an existing backup",
    responses(
        (status = 200, description = "Ok", content_type = "application/octet-stream"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server, backup storage or the backup does not exist"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("backup_storage" = String, description = "The id of the storage, in which the backup is stored"),
        ("backup_id" = i64, description = "The id of the backup"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_download_backup(
    State(state): State<DaemonState>,
    Path((server_id, backup_storage, backup_id)): Path<(u32, String, i64)>,
) -> Result<(HeaderMap, Body), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    storage::backup::backup_to_stream(&state, storage, backup_id, &backup_storage).await
}

#[cfg_attr(feature = "user", utoipa::path(get,
    path = "/user/backup/{server_id}/{backup_storage}/{backup_id}",
    tag = "backup",
    description = "Downloads an existing backup (User Request)",
    responses(
        (status = 200, description = "Ok", content_type = "application/octet-stream"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server, backup storage or the backup does not exist"),
        (status = 401, description = "The proof of the permission for the user request is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("backup_storage" = String, description = "The id of the storage, in which the backup is stored"),
        ("backup_id" = i64, description = "The id of the backup"),
        ("timestamp" = u64, Query, description = "The time the user request got permitted as Utc timestamp (milliseconds)"),
        ("verify" = String, Query, description = "The signature to proof that the user request was permitted")
   )
))]
#[cfg(feature = "user")]
pub(crate) async fn server_user_download_backup(
    State(state): State<DaemonState>,
    Path((server_id, backup_storage, backup_id)): Path<(u32, String, i64)>,
    axum::extract::Query(crate::web::user::GenericUserQuery { timestamp, verify }): axum::extract::Query<crate::web::user::GenericUserQuery>,
) -> Result<(HeaderMap, Body), StatusCode> {
    crate::web::user::authenticate_user(
        "backup",
        server_id,
        &state,
        &timestamp,
        &verify,
        Some(format!("-{}-{}", backup_storage, backup_id)),
    )
    .await?;
    let storage = StorageIdentifier::Primary(server_id);
    let body =
        storage::backup::backup_to_stream(&state, storage, backup_id, &backup_storage).await?;
    Ok(body)
}

#[utoipa::path(get,
    path = "/backup/{server_id}/rt",
    tag = "backup",
    description = "Downloads the contents of a server in realtime (without creating a backup)",
    responses(
        (status = 200, description = "Ok", content_type = "application/octet-stream"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server does not exist"),
        (status = 401, description = "Your authorization token is invalid"),
        (status = 400, description = "The compression is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("Authorization", Header, description = "The token used for authentication"),
        ("compression" = String, description = "The compression that the realtime backup should have")
   )
)]
pub(crate) async fn server_backup_realtime(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(CompressionQuery { compression }): Query<CompressionQuery>,
) -> Result<(HeaderMap, Body), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    storage::backup::stream_realtime(&state, storage, compression).await
}

#[cfg_attr(feature = "user", utoipa::path(get,
    path = "/user/backup/{server_id}/rt",
    tag = "backup",
    description = "Downloads the contents of a server in realtime (without creating a backup, User Request)",
    responses(
        (status = 200, description = "Ok", content_type = "application/octet-stream"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server does not exist"),
        (status = 401, description = "The proof of the permission for the user request is invalid"),
        (status = 400, description = "The compression is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("timestamp" = u64, Query, description = "The time the user request got permitted as Utc timestamp (milliseconds)"),
        ("verify" = String, Query, description = "The signature to proof that the user request was permitted"),
        ("compression" = String, Query, description = "The compression that the realtime backup should have")
   )
))]
#[cfg(feature = "user")]
pub(crate) async fn server_user_backup_realtime(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    axum::extract::Query(UserCompressionQuery {
        timestamp,
        verify,
        compression,
    }): axum::extract::Query<UserCompressionQuery>,
) -> Result<(HeaderMap, Body), StatusCode> {
    crate::web::user::authenticate_user(
        "backup_rt",
        server_id,
        &state,
        &timestamp,
        &verify,
        Some(format!("-{}", compression)),
    )
    .await?;
    let storage = StorageIdentifier::Primary(server_id);
    storage::backup::stream_realtime(&state, storage, compression).await
}

#[utoipa::path(delete,
    path = "/backup/{server_id}/{backup_storage}/{backup_id}",
    tag = "backup",
    description = "Deletes an existing backup",
    responses(
        (status = 200, description = "Backup was deleted successfully"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server, backup storage or the backup does not exist"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("backup_storage" = String, description = "The id of the storage, in which the backup is stored"),
        ("backup_id" = i64, description = "The id of the backup"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_delete_backup(
    State(state): State<DaemonState>,
    Path((server_id, backup_storage, backup_id)): Path<(u32, String, i64)>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    storage::backup::delete_backup(&state, storage, backup_id, &backup_storage).await
}

#[utoipa::path(get,
    path = "/backup/{server_id}",
    tag = "backup",
    description = "Lists all backups of an existing server",
    responses(
        (status = 200, description = "Ok", body = Vec<BackupResponseEntry>, content_type = "application/json"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server does not exist"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_list_backups(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<Json<Vec<BackupResponseEntry>>, StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    Ok(Json(storage::backup::list_backups(&state, storage).await?))
}

#[utoipa::path(get,
    path = "/backup/{server_id}/{backup_storage}/{backup_id}/restore",
    tag = "backup",
    description = "Restores an existing backup",
    responses(
        (status = 200, description = "Ok"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server, backup storage or the backup does not exist"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("backup_storage" = String, description = "The id of the storage, in which the backup is stored"),
        ("backup_id" = i64, description = "The id of the backup"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_restore_backup(
    State(state): State<DaemonState>,
    Path((server_id, backup_storage, backup_id)): Path<(u32, String, i64)>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    storage::backup::restore_backup(&state, storage, backup_id, &backup_storage).await
}

#[utoipa::path(patch,
    path = "/backup/{server_id}/{backup_storage}/{backup_id}",
    tag = "backup",
    description = "Sets the name of an existing backup",
    request_body(
        content = BackupUpdatePayload,
        description = "The backup update",
        content_type = "application/json",
    ),
    responses(
        (status = 200, description = "Ok"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server, backup storage or the backup does not exist"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("backup_storage" = String, description = "The id of the storage, in which the backup is stored"),
        ("backup_id" = i64, description = "The id of the backup"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_update_backup(
    State(state): State<DaemonState>,
    Path((server_id, backup_storage, backup_id)): Path<(u32, String, i64)>,
    Json(BackupUpdatePayload { name }): Json<BackupUpdatePayload>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    storage::backup::set_backup_name(&state, storage, backup_id, &backup_storage, name).await
}
