use crate::Daemon;
use crate::common::payload::files::FileResponse;
use crate::files::{
    copy_file, create_archive, delete_file, download_file, list_files, mkdir_file, rename_file,
    stat_file, symlink_file, unpack_archive, write_file, write_file_by_template, write_file_by_url,
};
use crate::storage::StorageIdentifier;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use daemon_common::payload::files::{
    PathCompressionSourceQuery, PathDestinationQuery, PathDirectoryQuery, PathQuery,
    PathSymlinkQuery, PathUrlQuery, TwoPathQuery,
};
use daemon_common::payload::server::ServerFileWrite;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub(crate) fn config() -> OpenApiRouter<Daemon> {
    OpenApiRouter::new()
        .routes(routes!(server_list_files))
        .routes(routes!(server_stat_file))
        .routes(routes!(server_rename_file))
        .routes(routes!(server_copy_file))
        .routes(routes!(server_download_file))
        .routes(routes!(server_write_file))
        .routes(routes!(server_write_file_by_url))
        .routes(routes!(server_write_file_by_template))
        .routes(routes!(server_delete_file))
        .routes(routes!(server_mkdir))
        .routes(routes!(server_symlink))
        .routes(routes!(server_unpack_archive))
        .routes(routes!(server_create_archive))
}

#[cfg(feature = "user")]
pub(crate) fn user_config() -> OpenApiRouter<Daemon> {
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
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathQuery { path }): Query<PathQuery>,
) -> Result<Json<Vec<FileResponse>>, StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    list_files(&state, storage, path).await.map(Json)
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
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathQuery { path }): Query<PathQuery>,
) -> Result<Json<FileResponse>, StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    stat_file(&state, storage, path).await.map(Json)
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
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
    Query(TwoPathQuery { from, to }): Query<TwoPathQuery>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    rename_file(&state, storage, from, to).await
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
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
    Query(TwoPathQuery { from, to }): Query<TwoPathQuery>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    copy_file(&state, storage, from, to).await
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
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathQuery { path }): Query<PathQuery>,
) -> Result<(Option<HeaderMap>, Body), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    download_file(&state, storage, path).await
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
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
    Query(daemon_common::payload::files::PathUserQuery {
        path,
        verify,
        timestamp,
    }): Query<daemon_common::payload::files::PathUserQuery>,
) -> Result<(Option<HeaderMap>, Body), StatusCode> {
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
    download_file(&state, storage, path).await
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
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathQuery { path }): Query<PathQuery>,
    request: Request,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    write_file(
        &state,
        storage,
        path,
        request.into_body().into_data_stream(),
    )
    .await
}

#[utoipa::path(get,
    path = "/files/{server_id}/url",
    tag = "files",
    description = "Writes a file using a url",
    responses(
        (status = 200, description = "Ok"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server or file does not exist"),
        (status = 403, description = "You tried to access files outside of the servers storage"),
        (status = 401, description = "Your authorization token is invalid"),
        (status = 400, description = "The url is invalid"),
        (status = 417, description = "Failed to access the url")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("path" = String, Query, description = "The path of the file"),
        ("url" = String, Query, description = "The url of the file to be downloaded"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_write_file_by_url(
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathUrlQuery { path, url }): Query<PathUrlQuery>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    write_file_by_url(
        &state,
        storage,
        path,
        url.parse().map_err(|_err| StatusCode::BAD_REQUEST)?,
    )
    .await
}

#[utoipa::path(post,
    path = "/files/{server_id}/template",
    tag = "files",
    description = "Writes a file using a template",
    responses(
        (status = 200, description = "Ok"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server or file does not exist"),
        (status = 403, description = "You tried to access files outside of the servers storage"),
        (status = 401, description = "Your authorization token is invalid"),
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("path" = String, Query, description = "The path of the file"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_write_file_by_template(
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathQuery { path }): Query<PathQuery>,
    Json(template): Json<ServerFileWrite>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    write_file_by_template(&state, storage, path, template).await
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
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathDirectoryQuery { path, recursive }): Query<PathDirectoryQuery>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    delete_file(&state, storage, path, recursive).await
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
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathDirectoryQuery { path, recursive }): Query<PathDirectoryQuery>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    mkdir_file(&state, storage, path, recursive).await
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
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathSymlinkQuery { path, content }): Query<PathSymlinkQuery>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    symlink_file(&state, storage, path, content).await
}

#[utoipa::path(get,
    path = "/files/{server_id}/archive/unpack",
    tag = "files",
    description = "Unpacks an existing archive",
    responses(
        (status = 200, description = "Ok"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server or file does not exist"),
        (status = 403, description = "You tried to access files outside of the servers storage"),
        (status = 401, description = "Your authorization token is invalid"),
        (status = 412, description = "The file is not an supported archive"),
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("path" = String, Query, description = "The path of the archive"),
        ("dest" = String, Query, description = "The path of the unpack destination"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_unpack_archive(
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathDestinationQuery { path, dest }): Query<PathDestinationQuery>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    unpack_archive(&state, storage, path, dest).await
}

#[utoipa::path(get,
    path = "/files/{server_id}/archive/create",
    tag = "files",
    description = "Creates an archive",
    responses(
        (status = 200, description = "Ok"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 503, description = "The storage of this server can't be accessed right now"),
        (status = 404, description = "The server or file does not exist"),
        (status = 403, description = "You tried to access files outside of the servers storage"),
        (status = 401, description = "Your authorization token is invalid"),
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("path" = String, Query, description = "The path of the archive"),
        ("src" = String, Query, description = "The path of the archive source"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_create_archive(
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathCompressionSourceQuery {
        path,
        src,
        compression,
    }): Query<PathCompressionSourceQuery>,
) -> Result<(), StatusCode> {
    let storage = StorageIdentifier::Primary(server_id);
    create_archive(&state, storage, path, src, compression).await
}
