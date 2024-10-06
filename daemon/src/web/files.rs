use crate::common::payload::files::FileResponse;
use crate::files::{copy_file, list_files, rename_file, stat_file, write_file};
use crate::storage::StorageIdentifier;
use crate::utils::server;
use crate::web::diesel_error_to_http;
use crate::DaemonState;
use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::Json;
use daemon_common::payload::files::{PathQuery, TwoPathQuery};
use std::ops::DerefMut;

pub(crate) async fn server_list_files(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathQuery { path }): Query<PathQuery>,
) -> Result<Json<Vec<FileResponse>>, StatusCode> {
    let _ = server(
        state.conn.lock().await.deref_mut(),
        &state.servers,
        server_id,
    )
    .await
    .map_err(diesel_error_to_http)?;
    let storage = StorageIdentifier::Primary(server_id);
    list_files(storage, path).await.map(Json)
}

pub(crate) async fn server_stat_file(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathQuery { path }): Query<PathQuery>,
) -> Result<Json<FileResponse>, StatusCode> {
    let _ = server(
        state.conn.lock().await.deref_mut(),
        &state.servers,
        server_id,
    )
    .await
    .map_err(diesel_error_to_http)?;
    let storage = StorageIdentifier::Primary(server_id);
    stat_file(storage, path).await.map(Json)
}

pub(crate) async fn server_rename_file(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(TwoPathQuery { from, to }): Query<TwoPathQuery>,
) -> Result<(), StatusCode> {
    let _ = server(
        state.conn.lock().await.deref_mut(),
        &state.servers,
        server_id,
    )
    .await
    .map_err(diesel_error_to_http)?;
    let storage = StorageIdentifier::Primary(server_id);
    rename_file(storage, from, to).await
}

pub(crate) async fn server_copy_file(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(TwoPathQuery { from, to }): Query<TwoPathQuery>,
) -> Result<(), StatusCode> {
    let _ = server(
        state.conn.lock().await.deref_mut(),
        &state.servers,
        server_id,
    )
    .await
    .map_err(diesel_error_to_http)?;
    let storage = StorageIdentifier::Primary(server_id);
    copy_file(storage, from, to).await
}

pub(crate) async fn server_write_file(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathQuery { path }): Query<PathQuery>,
    request: Request,
) -> Result<(), StatusCode> {
    let _ = server(
        state.conn.lock().await.deref_mut(),
        &state.servers,
        server_id,
    )
    .await
    .map_err(diesel_error_to_http)?;
    let storage = StorageIdentifier::Primary(server_id);
    write_file(storage, path, request.into_body().into_data_stream()).await
}
