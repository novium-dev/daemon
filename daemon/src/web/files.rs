use crate::common::payload::files::FileResponse;
use crate::files::{
    copy_file, delete_file, download_file, list_files, mkdir_file, rename_file, stat_file,
    symlink_file, write_file,
};
use crate::storage::StorageIdentifier;
use crate::utils::server;
use crate::web::diesel_error_to_http;
use crate::DaemonState;
use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::Json;
use daemon_common::payload::files::{
    PathDirectoryQuery, PathQuery, PathSymlinkQuery, TwoPathQuery,
};
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

pub(crate) async fn server_download_file(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathQuery { path }): Query<PathQuery>,
) -> Result<Body, StatusCode> {
    let _ = server(
        state.conn.lock().await.deref_mut(),
        &state.servers,
        server_id,
    )
    .await
    .map_err(diesel_error_to_http)?;
    let storage = StorageIdentifier::Primary(server_id);
    download_file(storage, path).await
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

pub(crate) async fn server_delete_file(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathDirectoryQuery { path, recursive }): Query<PathDirectoryQuery>,
) -> Result<(), StatusCode> {
    let _ = server(
        state.conn.lock().await.deref_mut(),
        &state.servers,
        server_id,
    )
    .await
    .map_err(diesel_error_to_http)?;
    let storage = StorageIdentifier::Primary(server_id);
    delete_file(storage, path, recursive).await
}

pub(crate) async fn server_mkdir(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathDirectoryQuery { path, recursive }): Query<PathDirectoryQuery>,
) -> Result<(), StatusCode> {
    let _ = server(
        state.conn.lock().await.deref_mut(),
        &state.servers,
        server_id,
    )
    .await
    .map_err(diesel_error_to_http)?;
    let storage = StorageIdentifier::Primary(server_id);
    mkdir_file(storage, path, recursive).await
}

pub(crate) async fn server_symlink(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathSymlinkQuery { path, content }): Query<PathSymlinkQuery>,
) -> Result<(), StatusCode> {
    let _ = server(
        state.conn.lock().await.deref_mut(),
        &state.servers,
        server_id,
    )
    .await
    .map_err(diesel_error_to_http)?;
    let storage = StorageIdentifier::Primary(server_id);
    symlink_file(storage, path, content).await
}
