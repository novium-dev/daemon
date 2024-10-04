use crate::ssh::sftp::IntoFileAttributes;
use crate::storage::StorageIdentifier;
use crate::utils::{parse_path, server};
use crate::web::diesel_error_to_http;
use crate::DaemonState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use crate::common::payload::files::FileResponse;
use russh_sftp::protocol::FileAttributes;
use serde::Deserialize;
use std::io;
use std::io::ErrorKind;
use std::path::PathBuf;

pub(crate) async fn server_list_files(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(PathQuery { path }): Query<PathQuery>,
) -> Result<Json<Vec<FileResponse>>, StatusCode> {
    let _ = server(state.conn.clone(), &state.servers, server_id)
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
    let _ = server(state.conn.clone(), &state.servers, server_id)
        .await
        .map_err(diesel_error_to_http)?;
    let storage = StorageIdentifier::Primary(server_id);
    stat_file(storage, path).await.map(Json)
}

pub(crate) async fn server_rename_file(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Query(RenamePathQuery { from, to }): Query<RenamePathQuery>,
) -> Result<(), StatusCode> {
    let _ = server(state.conn.clone(), &state.servers, server_id)
        .await
        .map_err(diesel_error_to_http)?;
    let storage = StorageIdentifier::Primary(server_id);
    rename_file(storage, from, to).await
}

async fn list_files(
    storage: StorageIdentifier,
    path: String,
) -> Result<Vec<FileResponse>, StatusCode> {
    let path = path0(storage, path, false).await?;

    let mut read_dir = tokio::fs::read_dir(&path).await.map_err(|err| match err {
        err if err.kind() == ErrorKind::NotFound => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    })?;

    let mut vec = Vec::new();
    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        let metadata = entry
            .metadata()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let is_symlink = metadata.is_symlink();
        let sftp_attrs = metadata.into_fileattributes();
        vec.push(
            sftp_to_http(
                entry.file_name().to_string_lossy().to_string(),
                entry.path(),
                is_symlink,
                sftp_attrs,
            )
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        )
    }

    Ok(vec)
}

async fn stat_file(storage: StorageIdentifier, path: String) -> Result<FileResponse, StatusCode> {
    let path = path0(storage, path, false).await?;

    let metadata = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(|err| match err {
            err if err.kind() == ErrorKind::NotFound => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        })?;

    let is_symlink = metadata.is_symlink();
    let sftp_attrs = metadata.into_fileattributes();
    let filename = match path.file_name() {
        None => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        Some(filename) => filename.to_string_lossy().to_string(),
    };
    sftp_to_http(filename, &path, is_symlink, sftp_attrs)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn rename_file(
    storage: StorageIdentifier,
    from: String,
    to: String,
) -> Result<(), StatusCode> {
    let from = path0(storage, from, false).await?;
    let to = path0(storage, to, true).await?;

    tokio::fs::rename(from, to).await.map_err(|err| match err {
        err if err.kind() == ErrorKind::NotFound => StatusCode::NOT_FOUND,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    })
}

async fn sftp_to_http(
    filename: String,
    filepath: impl AsRef<std::path::Path>,
    is_symlink: bool,
    attrs: FileAttributes,
) -> Result<FileResponse, io::Error> {
    let mut path = None;
    if is_symlink {
        path = Some(
            tokio::fs::read_link(filepath)
                .await?
                .to_string_lossy()
                .to_string(),
        )
    }
    Ok(FileResponse {
        name: filename,
        mode: attrs.permissions.unwrap(),
        uid: attrs.uid.unwrap(),
        gid: attrs.gid.unwrap(),
        size: attrs.size.unwrap_or_default(),
        path,
    })
}

async fn path0(
    storage: StorageIdentifier,
    mut path: String,
    check_parent: bool,
) -> Result<PathBuf, StatusCode> {
    let (components, ends_with_slash) =
        parse_path(&mut path).map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut components: Vec<String> = components.into_iter().map(String::from).collect();

    if !components.is_empty() && ends_with_slash && !components[components.len() - 1].ends_with('/')
    {
        let index = components.len() - 1;
        components[index] = format!("{}/", components[index]);
    }
    let mut r#final = None;
    if !components.is_empty() && check_parent {
        r#final = Some(components.remove(components.len() - 1));
    }

    let mut base = storage.as_pathbuf();
    let mut path = base.clone();
    base = tokio::fs::canonicalize(base)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    for component in components {
        path = path.join(component);
    }

    path = tokio::fs::canonicalize(path)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if path.strip_prefix(base).is_err() {
        return Err(StatusCode::BAD_REQUEST);
    }
    if let Some(component) = r#final {
        path = path.join(component);
    }
    Ok(path)
}

#[derive(Deserialize)]
pub(crate) struct PathQuery {
    path: String,
}

#[derive(Deserialize)]
pub(crate) struct RenamePathQuery {
    from: String,
    to: String,
}
