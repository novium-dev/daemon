use crate::storage::StorageIdentifier;
use crate::utils::{parse_path, storage_read, StorageError};
use crate::DaemonState;
use axum::body::{Body, BodyDataStream};
use axum::http::StatusCode;
use daemon_common::files::FileMode;
use daemon_common::payload::files::FileResponse;
use diesel_async::AsyncMysqlConnection;
use futures_util::TryStreamExt;
use std::fs::Metadata;
use std::io;
use std::io::ErrorKind;
use std::os::unix::prelude::*;
use std::path::PathBuf;
use tokio::fs::OpenOptions;
use tokio::io::BufWriter;
use tokio::sync::OwnedMutexGuard;
use tokio_util::io::{ReaderStream, StreamReader};

pub(crate) async fn list_files(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    storage: StorageIdentifier,
    path: String,
) -> Result<Vec<FileResponse>, StatusCode> {
    let (_path, lock) = storage_read(&state.config, &mut conn, &state.storages, storage, true)
        .await
        .map_err(StorageError::into_http)?;
    drop(conn);
    let files = list_files0(storage, path).await?;
    drop(lock);
    Ok(files)
}

#[inline(always)]
async fn list_files0(
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
        let mut link_path = None;
        if metadata.is_symlink() {
            link_path = Some(
                tokio::fs::read_link(&path)
                    .await
                    .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?
                    .to_string_lossy()
                    .to_string(),
            )
        }
        vec.push(
            metadata.into_fileresponse(entry.file_name().to_string_lossy().to_string(), link_path),
        );
    }

    Ok(vec)
}

pub(crate) async fn stat_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    storage: StorageIdentifier,
    path: String,
) -> Result<FileResponse, StatusCode> {
    let (_path, lock) = storage_read(&state.config, &mut conn, &state.storages, storage, true)
        .await
        .map_err(StorageError::into_http)?;
    drop(conn);
    let file = stat_file0(storage, path).await?;
    drop(lock);
    Ok(file)
}

#[inline(always)]
async fn stat_file0(storage: StorageIdentifier, path: String) -> Result<FileResponse, StatusCode> {
    let path = path0(storage, path, false).await?;

    let metadata = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(|err| match err {
            err if err.kind() == ErrorKind::NotFound => StatusCode::NOT_FOUND,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        })?;

    let filename = match path.file_name() {
        None => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        Some(filename) => filename.to_string_lossy().to_string(),
    };
    let mut link_path = None;
    if metadata.is_symlink() {
        link_path = Some(
            tokio::fs::read_link(&path)
                .await
                .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?
                .to_string_lossy()
                .to_string(),
        )
    }
    Ok(metadata.into_fileresponse(filename, link_path))
}

pub(crate) async fn rename_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    storage: StorageIdentifier,
    from: String,
    to: String,
) -> Result<(), StatusCode> {
    let (_path, lock) = storage_read(&state.config, &mut conn, &state.storages, storage, true)
        .await
        .map_err(StorageError::into_http)?;
    drop(conn);
    rename_file0(storage, from, to).await?;
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn rename_file0(
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

pub(crate) async fn copy_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    storage: StorageIdentifier,
    from: String,
    to: String,
) -> Result<(), StatusCode> {
    let (_path, lock) = storage_read(&state.config, &mut conn, &state.storages, storage, true)
        .await
        .map_err(StorageError::into_http)?;
    drop(conn);
    copy_file0(storage, from, to).await?;
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn copy_file0(
    storage: StorageIdentifier,
    from: String,
    to: String,
) -> Result<(), StatusCode> {
    let from = path0(storage, from, false).await?;
    let to = path0(storage, to, true).await?;
    tokio::fs::copy(from, to)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(())
}

pub(crate) async fn download_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    storage: StorageIdentifier,
    path: String,
) -> Result<Body, StatusCode> {
    let (_path, lock) = storage_read(&state.config, &mut conn, &state.storages, storage, true)
        .await
        .map_err(StorageError::into_http)?;
    drop(conn);
    let body = download_file0(storage, path).await?;
    drop(lock);
    Ok(body)
}

#[inline(always)]
async fn download_file0(storage: StorageIdentifier, path: String) -> Result<Body, StatusCode> {
    let path = path0(storage, path, true).await?;
    let file = OpenOptions::new()
        .read(true)
        .open(&path)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    let stream = ReaderStream::new(file);
    let body = axum::body::Body::from_stream(stream);
    Ok(body)
}

pub(crate) async fn write_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    storage: StorageIdentifier,
    path: String,
    stream: BodyDataStream,
) -> Result<(), StatusCode> {
    let (_path, lock) = storage_read(&state.config, &mut conn, &state.storages, storage, true)
        .await
        .map_err(StorageError::into_http)?;
    drop(conn);
    write_file0(storage, path, stream).await?;
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn write_file0(
    storage: StorageIdentifier,
    path: String,
    stream: BodyDataStream,
) -> Result<(), StatusCode> {
    let path = path0(storage, path, true).await?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut reader =
        StreamReader::new(stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err)));
    let mut writer = BufWriter::new(file);
    tokio::io::copy(&mut reader, &mut writer)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(())
}

pub(crate) async fn mkdir_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    storage: StorageIdentifier,
    path: String,
    recursive: bool,
) -> Result<(), StatusCode> {
    let (_path, lock) = storage_read(&state.config, &mut conn, &state.storages, storage, true)
        .await
        .map_err(StorageError::into_http)?;
    drop(conn);
    mkdir_file0(storage, path, recursive).await?;
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn mkdir_file0(
    storage: StorageIdentifier,
    path: String,
    recursive: bool,
) -> Result<(), StatusCode> {
    if recursive {
        // unstable, difficult to check -> avoid security risks
        return Err(StatusCode::NOT_IMPLEMENTED);
    }
    let path = path0(storage, path, true).await?;
    tokio::fs::create_dir(path).await.map_err(|err| {
        if err.kind() == ErrorKind::AlreadyExists {
            StatusCode::OK
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    })
}

pub(crate) async fn symlink_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    storage: StorageIdentifier,
    path: String,
    content: String,
) -> Result<(), StatusCode> {
    let (_path, lock) = storage_read(&state.config, &mut conn, &state.storages, storage, true)
        .await
        .map_err(StorageError::into_http)?;
    drop(conn);
    symlink_file0(storage, path, content).await?;
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn symlink_file0(
    storage: StorageIdentifier,
    path: String,
    content: String,
) -> Result<(), StatusCode> {
    let path = path0(storage, path, true).await?;
    tokio::fs::symlink(PathBuf::from(content), path)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)
}

pub(crate) async fn delete_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    storage: StorageIdentifier,
    path: String,
    recursive: bool,
) -> Result<(), StatusCode> {
    let (_path, lock) = storage_read(&state.config, &mut conn, &state.storages, storage, true)
        .await
        .map_err(StorageError::into_http)?;
    drop(conn);
    delete_file0(storage, path, recursive).await?;
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn delete_file0(
    storage: StorageIdentifier,
    path: String,
    recursive: bool,
) -> Result<(), StatusCode> {
    let path = path0(storage, path, true).await?;
    if recursive {
        tokio::fs::remove_dir_all(path).await
    } else {
        tokio::fs::remove_file(path).await
    }
    .map_err(|err| match err {
        err if err
            .raw_os_error() // TODO: feature io_error_more will be available with rust 1.82 or 1.83. Use this instead if it is available
            .is_some_and(|err| err == libc::EISDIR || err == libc::ENOTDIR) =>
        {
            StatusCode::BAD_REQUEST
        }
        _err => StatusCode::INTERNAL_SERVER_ERROR,
    })?;
    Ok(())
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

pub trait IntoFileResponse {
    fn into_fileresponse(self, filename: String, path: Option<String>) -> FileResponse;
}

impl IntoFileResponse for Metadata {
    fn into_fileresponse(self, filename: String, path: Option<String>) -> FileResponse {
        let mut r#type = 0;
        let file_type = self.file_type();
        if file_type.is_dir() {
            r#type |= FileMode::DIR.bits();
        } else if file_type.is_symlink() {
            r#type |= FileMode::LNK.bits();
        } else if file_type.is_socket() {
            r#type |= FileMode::SOCK.bits();
        } else if file_type.is_block_device() {
            r#type |= FileMode::BLK.bits();
        } else if file_type.is_char_device() {
            r#type |= FileMode::CHR.bits();
        } else if file_type.is_fifo() {
            r#type |= FileMode::FIFO.bits();
        } else {
            r#type |= FileMode::REG.bits();
        }
        FileResponse {
            name: filename,
            path,
            mode: self.mode() | r#type,
            size: self.size(),
            uid: self.uid(),
            gid: self.gid(),
        }
    }
}
