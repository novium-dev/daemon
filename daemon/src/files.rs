use std::io;
use std::io::ErrorKind;
use std::os::unix::prelude::*;
use std::path::PathBuf;

use axum::body::{Body, BodyDataStream};
use axum::http;
use axum::http::{HeaderMap, StatusCode};
use cap_async_std::ambient_authority;
use cap_async_std::fs::{Dir, FileTypeExt, Metadata, MetadataExt, OpenOptions};
use diesel_async::AsyncMysqlConnection;
use futures_util::TryStreamExt;
use tokio::io::BufWriter;
use tokio::sync::OwnedMutexGuard;
use tokio_util::io::{ReaderStream, StreamReader};

use crate::config::StorageEntry;
use crate::storage::StorageIdentifier;
use crate::utils::{path_relative, storage_read, StorageError};
use crate::DaemonState;
use daemon_common::files::FileMode;
use daemon_common::payload::files::FileResponse;

fn err(err: io::Error) -> StatusCode {
    println!("Error {:?}", err);
    match err.kind() {
        ErrorKind::NotFound => StatusCode::NOT_FOUND,
        ErrorKind::PermissionDenied => StatusCode::FORBIDDEN,
        ErrorKind::IsADirectory | ErrorKind::NotADirectory => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub(crate) async fn list_files(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    id: StorageIdentifier,
    path: String,
) -> Result<Vec<FileResponse>, StatusCode> {
    let (_path, lock, _, storage) =
        storage_read(&state.config, &mut conn, &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let path = path_relative(path);
    drop(conn);
    let files = list_files0(id, storage, path).await?;
    drop(lock);
    Ok(files)
}

#[inline(always)]
async fn list_files0(
    id: StorageIdentifier,
    storage: &StorageEntry,
    path: PathBuf,
) -> Result<Vec<FileResponse>, StatusCode> {
    let storage = storage_path(id, storage).await?;

    let read_dir = storage.read_dir(&path).await.map_err(err)?;
    let mut vec = Vec::new();
    for entry in read_dir {
        let entry = entry.map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
        let metadata = entry
            .metadata()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut link_path = None;
        if metadata.is_symlink() {
            link_path = Some(
                storage
                    .read_link(PathBuf::from(&path).join(entry.file_name()))
                    .await
                    .map_err(err)?
                    .to_string_lossy()
                    .to_string(),
            );
        }

        let relative_path = path.join(entry.file_name());
        vec.push(metadata.into_fileresponse(
            entry.file_name().to_string_lossy().to_string(),
            relative_path.to_string_lossy().to_string(),
            link_path,
        ));
    }

    Ok(vec)
}

pub(crate) async fn stat_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    id: StorageIdentifier,
    path: String,
) -> Result<FileResponse, StatusCode> {
    let (_path, lock, _, storage) =
        storage_read(&state.config, &mut conn, &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    drop(conn);
    let path = path_relative(path);
    let file = stat_file0(id, storage, path).await?;
    drop(lock);
    Ok(file)
}

#[inline(always)]
async fn stat_file0(
    id: StorageIdentifier,
    storage: &StorageEntry,
    path: PathBuf,
) -> Result<FileResponse, StatusCode> {
    let storage = storage_path(id, storage).await?;
    let metadata = storage.symlink_metadata(&path).await.map_err(err)?;

    let filename = match path.file_name() {
        None => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        Some(filename) => filename.to_string_lossy().to_string(),
    };
    let mut link_path = None;
    if metadata.is_symlink() {
        link_path = Some(
            storage
                .read_link(path.clone())
                .await
                .map_err(err)?
                .to_string_lossy()
                .to_string(),
        );
    }

    let relative_path = path.join(filename.clone());
    Ok(metadata.into_fileresponse(
        filename,
        relative_path.to_string_lossy().to_string(),
        link_path,
    ))
}

pub(crate) async fn rename_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    id: StorageIdentifier,
    from: String,
    to: String,
) -> Result<(), StatusCode> {
    let (_path, lock, _, storage) =
        storage_read(&state.config, &mut conn, &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    drop(conn);
    let from = path_relative(from);
    let to = path_relative(to);
    let storage = storage_path(id, storage).await?;
    storage.rename(from, &storage, to).await.map_err(err)?;
    drop(lock);
    Ok(())
}

pub(crate) async fn copy_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    id: StorageIdentifier,
    from: String,
    to: String,
) -> Result<(), StatusCode> {
    let (_path, lock, _, storage) =
        storage_read(&state.config, &mut conn, &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    drop(conn);
    let from = path_relative(from);
    let to = path_relative(to);
    let storage = storage_path(id, storage).await?;
    storage.copy(from, &storage, to).await.map_err(err)?;
    drop(lock);
    Ok(())
}

pub(crate) async fn download_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    id: StorageIdentifier,
    path: String,
) -> Result<(Option<HeaderMap>, Body), StatusCode> {
    let (_path, lock, _, storage) =
        storage_read(&state.config, &mut conn, &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    drop(conn);
    let path = path_relative(path);
    let storage = storage_path(id, storage).await?;
    let headers = path
        .file_name()
        .map(|str| str.to_string_lossy())
        .map(|filename| {
            HeaderMap::from_iter([(
                http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename)
                    .parse()
                    .unwrap(),
            )])
        });
    let fd: OwnedFd = storage.open(path).await.map_err(err)?.into_std().into();
    let stream = ReaderStream::new(tokio::fs::File::from_std(std::fs::File::from(fd)));
    let body = axum::body::Body::from_stream(stream);
    drop(lock);
    Ok((headers, body))
}

pub(crate) async fn write_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    id: StorageIdentifier,
    path: String,
    stream: BodyDataStream,
) -> Result<(), StatusCode> {
    let (_path, lock, _, storage) =
        storage_read(&state.config, &mut conn, &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    drop(conn);
    let path = path_relative(path);
    let storage = storage_path(id, storage).await?;
    let mut reader =
        StreamReader::new(stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err)));
    let fd: OwnedFd = storage
        .open_with(
            path,
            OpenOptions::new().create(true).truncate(true).write(true),
        )
        .await
        .map_err(err)?
        .into_std()
        .into();
    let mut writer = BufWriter::new(tokio::fs::File::from_std(std::fs::File::from(fd)));
    tokio::io::copy(&mut reader, &mut writer)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    drop(lock);
    Ok(())
}

pub(crate) async fn mkdir_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    id: StorageIdentifier,
    path: String,
    recursive: bool,
) -> Result<(), StatusCode> {
    let (_path, lock, _, storage) =
        storage_read(&state.config, &mut conn, &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    drop(conn);
    let path = path_relative(path);
    let storage = storage_path(id, storage).await?;
    tokio::task::spawn_blocking(move || {
        if recursive {
            storage.create_dir_all(&path)
        } else {
            storage.create_dir(&path)
        }
    })
    .await
    .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(err)?;
    drop(lock);
    Ok(())
}

pub(crate) async fn symlink_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    id: StorageIdentifier,
    path: String,
    content: String,
) -> Result<(), StatusCode> {
    let (_path, lock, _, storage) =
        storage_read(&state.config, &mut conn, &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    drop(conn);
    let path = path_relative(path);
    let storage = storage_path(id, storage).await?;
    storage.symlink(content, path).await.map_err(err)?;
    drop(lock);
    Ok(())
}

pub(crate) async fn delete_file(
    state: &DaemonState,
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    id: StorageIdentifier,
    path: String,
    recursive: bool,
) -> Result<(), StatusCode> {
    let (_path, lock, _, storage) =
        storage_read(&state.config, &mut conn, &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    drop(conn);
    let path = path_relative(path);
    let storage = storage_path(id, storage).await?;
    if recursive {
        storage.remove_dir_all(path).await
    } else {
        storage.remove_file(path).await
    }
    .map_err(err)?;
    drop(lock);
    Ok(())
}

async fn storage_path(id: StorageIdentifier, storage: &StorageEntry) -> Result<Dir, StatusCode> {
    let base = PathBuf::from(storage.path()).join(id.as_pathbuf());
    Dir::open_ambient_dir(&base, ambient_authority())
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)
}

pub trait IntoFileResponse {
    fn into_fileresponse(
        self,
        filename: String,
        relative_path: String,
        path: Option<String>,
    ) -> FileResponse;
}

impl IntoFileResponse for Metadata {
    fn into_fileresponse(
        self,
        filename: String,
        relative_path: String,
        path: Option<String>,
    ) -> FileResponse {
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
            relative_path,
            size: self.size(),
            uid: self.uid(),
            gid: self.gid(),
        }
    }
}
