use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::Json;
use daemon_common::payload::files::{
    PathDirectoryQuery, PathQuery, PathSymlinkQuery, TwoPathQuery,
};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::common::payload::files::FileResponse;
use crate::files::{
    copy_file, delete_file, download_file, list_files, mkdir_file, rename_file, stat_file,
    symlink_file, write_file,
};
use crate::storage::StorageIdentifier;
use crate::DaemonState;

pub(crate) fn config() -> OpenApiRouter<DaemonState> {
    OpenApiRouter::new()
        .routes(routes!(server_list_files))
        .routes(routes!(server_stat_file))
        .routes(routes!(server_rename_file))
        .routes(routes!(server_copy_file))
        .routes(routes!(server_download_file))
        .routes(routes!(server_write_file))
        .routes(routes!(server_delete_file))
        .routes(routes!(server_mkdir))
        .routes(routes!(server_symlink))
}

pub(crate) fn user_config() -> OpenApiRouter<DaemonState> {
    OpenApiRouter::new().routes(routes!(server_user_download_file))
}

#[utoipa::path(get,
    path = "/files/{server_id}/list",
    tag = "files",
    description = "List files inside a directory",
    responses(
        (status = 200, description = "Ok", body = Vec<FileResponse>, content_type = "application/json"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 403, description = "You tried to access files outside of the servers storage"),
        (status = 404, description = "The server or directory does not exist"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("path" = String, Query, description = "The path of the directory in which the files should be listed"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_list_files(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathQuery { path }): Query<PathQuery>,
) -> Result<Json<Vec<FileResponse>>, StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    list_files(&state, state.conn.clone().lock_owned().await, storage, path)
        .await
        .map(Json)
}

#[utoipa::path(get,
    path = "/files/{server_id}/stat",
    tag = "files",
    description = "Gets the info about a file",
    responses(
        (status = 200, description = "Ok", body = FileResponse, content_type = "application/json"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server or file does not exist"),
        (status = 403, description = "You tried to access files outside of the servers storage"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("path" = String, Query, description = "The path of the file or directory"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_stat_file(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathQuery { path }): Query<PathQuery>,
) -> Result<Json<FileResponse>, StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    stat_file(&state, state.conn.clone().lock_owned().await, storage, path)
        .await
        .map(Json)
}

#[utoipa::path(get,
    path = "/files/{server_id}/rename",
    tag = "files",
    description = "Renames/Moves a file",
    responses(
        (status = 200, description = "File or directory was renamed successfully"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server or file does not exist"),
        (status = 403, description = "You tried to access files outside of the servers storage"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("from" = String, Query, description = "The old path of the file"),
        ("to" = String, Query, description = "The new path of the file"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_rename_file(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(TwoPathQuery { from, to }): Query<TwoPathQuery>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    rename_file(
        &state,
        state.conn.clone().lock_owned().await,
        storage,
        from,
        to,
    )
    .await
}

#[utoipa::path(get,
    path = "/files/{server_id}/copy",
    tag = "files",
    description = "Copies a file",
    responses(
        (status = 200, description = "File was copied successfully"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server or file does not exist"),
        (status = 403, description = "You tried to access files outside of the servers storage"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("from" = String, Query, description = "The old path of the file"),
        ("to" = String, Query, description = "The path of the copied file"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_copy_file(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(TwoPathQuery { from, to }): Query<TwoPathQuery>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    copy_file(
        &state,
        state.conn.clone().lock_owned().await,
        storage,
        from,
        to,
    )
    .await
}

#[utoipa::path(get,
    path = "/files/{server_id}/file",
    tag = "files",
    description = "Downloads a file",
    responses(
        (status = 200, description = "Ok", content_type = "application/octet-stream"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server or file does not exist"),
        (status = 403, description = "You tried to access files outside of the servers storage"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("path" = String, Query, description = "The path of the file"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_download_file(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathQuery { path }): Query<PathQuery>,
) -> Result<Body, StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    download_file(&state, state.conn.clone().lock_owned().await, storage, path).await
}

#[cfg_attr(feature = "user", utoipa::path(get,
    path = "/user/files/{server_id}/file",
    tag = "files",
    description = "Downloads a file (User Request)",
    responses(
        (status = 200, description = "Ok", content_type = "application/octet-stream"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server or file does not exist"),
        (status = 403, description = "You tried to access files outside of the servers storage"),
        (status = 401, description = "The proof of the permission for the user request is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("path" = String, Query, description = "The path of the file"),
        ("timestamp" = u64, Query, description = "The time the user request got permitted as Utc timestamp (milliseconds)"),
        ("verify" = String, Query, description = "The signature to proof that the user request was permitted")
   )
))]
#[cfg(feature = "user")]
pub(crate) async fn server_user_download_file(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(daemon_common::payload::files::PathUserQuery {
        path,
        verify,
        timestamp,
    }): Query<daemon_common::payload::files::PathUserQuery>,
) -> Result<Body, StatusCode> {
    crate::web::user::authenticate_user(
        "file",
        server_id,
        &state,
        &timestamp,
        &verify,
        Some(format!("-{}", path)),
    )
    .await?;
    let storage = StorageIdentifier::Primary(server_id);
    download_file(&state, state.conn.clone().lock_owned().await, storage, path).await
}

#[utoipa::path(put,
    path = "/files/{server_id}/file",
    tag = "files",
    description = "Uploads a file",
    request_body(
        content_type = "application/octet-stream"
    ),
    responses(
        (status = 200, description = "Ok"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server or file does not exist"),
        (status = 403, description = "You tried to access files outside of the servers storage"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("path" = String, Query, description = "The path of the file"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_write_file(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathQuery { path }): Query<PathQuery>,
    request: Request,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    write_file(
        &state,
        state.conn.clone().lock_owned().await,
        storage,
        path,
        request.into_body().into_data_stream(),
    )
    .await
}

#[utoipa::path(get,
    path = "/files/{server_id}/delete",
    tag = "files",
    description = "Deletes a file",
    responses(
        (status = 200, description = "File or directory was deleted successfully"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server or file does not exist"),
        (status = 403, description = "You tried to access files outside of the servers storage"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("path" = String, Query, description = "The path of the file"),
        ("recursive" = bool, Query, description = "If the file or the directory should be deleted"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_delete_file(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathDirectoryQuery { path, recursive }): Query<PathDirectoryQuery>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    delete_file(
        &state,
        state.conn.clone().lock_owned().await,
        storage,
        path,
        recursive,
    )
    .await
}

#[utoipa::path(get,
    path = "/files/{server_id}/mkdir",
    tag = "files",
    description = "Creates a new directory",
    responses(
        (status = 200, description = "Directory was created successfully"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server or file does not exist"),
        (status = 403, description = "You tried to access files outside of the servers storage"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("path" = String, Query, description = "The path of the new directory"),
        ("recursive" = bool, Query, description = "If the parent directory does not exist and also needs to be created"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_mkdir(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathDirectoryQuery { path, recursive }): Query<PathDirectoryQuery>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    mkdir_file(
        &state,
        state.conn.clone().lock_owned().await,
        storage,
        path,
        recursive,
    )
    .await
}

#[utoipa::path(get,
    path = "/files/{server_id}/symlink",
    tag = "files",
    description = "Creates a new symlink",
    responses(
        (status = 200, description = "Symlink was created successfully"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server or file does not exist"),
        (status = 403, description = "You tried to access files outside of the servers storage"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("path" = String, Query, description = "The path of the new symlink"),
        ("content" = String, Query, description = "The destination path of the link"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_symlink(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathSymlinkQuery { path, content }): Query<PathSymlinkQuery>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    symlink_file(
        &state,
        state.conn.clone().lock_owned().await,
        storage,
        path,
        content,
    )
    .await
}
