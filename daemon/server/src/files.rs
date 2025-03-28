use std::collections::HashSet;
use std::io;
use std::io::ErrorKind;
use std::ops::Deref;
use std::os::unix::prelude::*;
use std::path::PathBuf;

use crate::DaemonState;
use crate::config::StorageEntry;
use crate::storage::StorageIdentifier;
use crate::storage::archive::compression::ArchiveCompression;
use crate::storage::archive::{TarSandboxedAppendExtension, TarSandboxedUnpackExtension};
use crate::storage::sync::{StorageError, StorageLockData, storage_read};
use crate::utils::path_relative;
use axum::body::{Body, BodyDataStream};
use axum::http;
use axum::http::{HeaderMap, StatusCode};
use cap_async_std::ambient_authority;
use cap_async_std::fs::{Dir, FileTypeExt, Metadata, MetadataExt, OpenOptions};
use daemon_common::files::FileMode;
use daemon_common::payload::files::FileResponse;
use daemon_common::payload::server::ServerFileWrite;
use futures_util::TryStreamExt;
use reqwest::Url;
use serde_json::json;
use tar::{Archive, Builder};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio_util::io::{ReaderStream, StreamReader};
use tracing::{debug, error, warn};

fn err(err: io::Error) -> StatusCode {
    match err.kind() {
        ErrorKind::NotFound => StatusCode::NOT_FOUND,
        ErrorKind::PermissionDenied => StatusCode::FORBIDDEN,
        ErrorKind::IsADirectory | ErrorKind::NotADirectory => StatusCode::BAD_REQUEST,
        ErrorKind::UnexpectedEof => StatusCode::PRECONDITION_FAILED,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub(crate) async fn list_files(
    state: &DaemonState,
    id: StorageIdentifier,
    path: String,
) -> Result<Vec<FileResponse>, StatusCode> {
    let StorageLockData { lock, storage, .. } =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;
    let files = list_files0(storage, path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory => debug!("Failed to list files: {:?}", err),
            _ => error!("Failed to list files: {:?}", err),
        })
        .map_err(err)?;
    drop(lock);
    Ok(files)
}

#[inline(always)]
async fn list_files0(storage: Dir, path: PathBuf) -> Result<Vec<FileResponse>, io::Error> {
    let read_dir = storage.read_dir(&path).await?;
    let mut vec = Vec::new();
    for entry in read_dir {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let mut link_path = None;
        let mut archive = false;

        let relative_path = path.join(entry.file_name());
        let file_name = entry.file_name().to_string_lossy().to_string();
        if metadata.is_symlink() {
            link_path = Some(
                storage
                    .read_link(PathBuf::from(&path).join(entry.file_name()))
                    .await?
                    .to_string_lossy()
                    .to_string(),
            );
        } else {
            archive = tokio::task::spawn_blocking(move || {
                let file = entry.open()?;
                let fd: OwnedFd = file.into_std().into();
                let file = std::fs::File::from(fd);
                if let Ok(Some(_)) = ArchiveCompression::detect_blocking(file) {
                    Ok::<_, io::Error>(true)
                } else {
                    Ok(false)
                }
            })
            .await??;
        }

        vec.push(metadata.into_fileresponse(
            file_name,
            relative_path.to_string_lossy().to_string(),
            link_path,
            archive,
        ));
    }

    Ok(vec)
}

pub(crate) async fn stat_file(
    state: &DaemonState,
    id: StorageIdentifier,
    path: String,
) -> Result<FileResponse, StatusCode> {
    let StorageLockData { lock, storage, .. } =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;
    let file = stat_file0(storage, path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory => debug!("Failed to stat file: {:?}", err),
            _ => error!("Failed to stat file: {:?}", err),
        })
        .map_err(err)?;
    drop(lock);
    Ok(file)
}

#[inline(always)]
async fn stat_file0(storage: Dir, path: PathBuf) -> Result<FileResponse, io::Error> {
    let metadata = storage.symlink_metadata(&path).await?;

    let filename = match path.file_name() {
        None => {
            return Err(io::Error::new(
                ErrorKind::Other,
                "Final component of path is missing",
            ));
        }
        Some(filename) => filename.to_string_lossy().to_string(),
    };
    let mut link_path = None;
    let mut archive = false;
    if metadata.is_symlink() {
        link_path = Some(
            storage
                .read_link(path.clone())
                .await?
                .to_string_lossy()
                .to_string(),
        );
    } else {
        let file = storage.open(&path).await?;
        archive = tokio::task::spawn_blocking(move || {
            let fd: OwnedFd = file.into_std().into();
            let file = std::fs::File::from(fd);
            if let Ok(Some(_)) = ArchiveCompression::detect_blocking(file) {
                Ok::<_, io::Error>(true)
            } else {
                Ok(false)
            }
        })
        .await??;
    }

    let relative_path = path.join(filename.clone());
    Ok(metadata.into_fileresponse(
        filename,
        relative_path.to_string_lossy().to_string(),
        link_path,
        archive,
    ))
}

pub(crate) async fn rename_file(
    state: &DaemonState,
    id: StorageIdentifier,
    from: String,
    to: String,
) -> Result<(), StatusCode> {
    let StorageLockData { lock, storage, .. } =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let from = path_relative(from);
    let to = path_relative(to);
    let (storage, _) = storage_path(id, &storage).await?;
    storage
        .rename(from, &storage, to)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory
            | ErrorKind::AlreadyExists => debug!("Failed to rename file: {:?}", err),
            _ => error!("Failed to rename file: {:?}", err),
        })
        .map_err(err)?;
    drop(lock);
    Ok(())
}

pub(crate) async fn copy_file(
    state: &DaemonState,
    id: StorageIdentifier,
    from: String,
    to: String,
) -> Result<(), StatusCode> {
    let StorageLockData { lock, storage, .. } =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let from = path_relative(from);
    let to = path_relative(to);
    let (storage, _) = storage_path(id, &storage).await?;
    storage
        .copy(from, &storage, to)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory
            | ErrorKind::AlreadyExists => debug!("Failed to copy file: {:?}", err),
            _ => error!("Failed to copy file: {:?}", err),
        })
        .map_err(err)?;
    drop(lock);
    Ok(())
}

pub(crate) async fn download_file(
    state: &DaemonState,
    id: StorageIdentifier,
    path: String,
) -> Result<(Option<HeaderMap>, Body), StatusCode> {
    let StorageLockData { lock, storage, .. } =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;
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
    let fd: OwnedFd = storage
        .open(path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory => debug!("Failed to download file: {:?}", err),
            _ => error!("Failed to download file: {:?}", err),
        })
        .map_err(err)?
        .into_std()
        .into();
    let stream = ReaderStream::new(tokio::fs::File::from_std(std::fs::File::from(fd)));
    let body = axum::body::Body::from_stream(stream);
    drop(lock);
    Ok((headers, body))
}

pub(crate) async fn write_file(
    state: &DaemonState,
    id: StorageIdentifier,
    path: String,
    stream: BodyDataStream,
) -> Result<(), StatusCode> {
    let StorageLockData { lock, storage, .. } =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;
    let mut reader =
        StreamReader::new(stream.map_err(|err| io::Error::new(io::ErrorKind::Other, err)));
    let fd: OwnedFd = storage
        .open_with(
            path,
            OpenOptions::new().create(true).truncate(true).write(true),
        )
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory => debug!("Failed to write file (open): {:?}", err),
            _ => error!("Failed to write file (open): {:?}", err),
        })
        .map_err(err)?
        .into_std()
        .into();
    let mut writer = BufWriter::new(tokio::fs::File::from_std(std::fs::File::from(fd)));
    tokio::io::copy(&mut reader, &mut writer)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory => debug!("Failed to write file: {:?}", err),
            _ => error!("Failed to write file: {:?}", err),
        })
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    drop(lock);
    Ok(())
}

pub(crate) async fn write_file_by_template(
    state: &DaemonState,
    id: StorageIdentifier,
    path: String,
    template: ServerFileWrite,
) -> Result<(), StatusCode> {
    let StorageLockData { lock, storage, .. } =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;

    let fd: OwnedFd = storage
        .open_with(path, OpenOptions::new().create(true).read(true).write(true))
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory => {
                debug!("Failed to write file by template (open): {:?}", err)
            }
            _ => error!("Failed to write file by template (open): {:?}", err),
        })
        .map_err(err)?
        .into_std()
        .into();
    let file = tokio::fs::File::from_std(std::fs::File::from(fd));
    template
        .apply(file)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory => debug!("Failed to write file by template: {:?}", err),
            _ => error!("Failed to write file by template: {:?}", err),
        })
        .map_err(err)?;

    drop(lock);
    Ok(())
}

pub(crate) async fn write_file_by_url(
    state: &DaemonState,
    id: StorageIdentifier,
    path: String,
    url: Url,
) -> Result<(), StatusCode> {
    let StorageLockData { lock, storage, .. } =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;
    let mut response = state
        .client
        .get(url.clone())
        .send()
        .await
        .inspect_err(|err| warn!("Failed to write file by url {} (request): {:?}", url, err))
        .map_err(|_err| StatusCode::EXPECTATION_FAILED)?
        .error_for_status()
        .inspect_err(|err| warn!("Failed to write file by url {} (status): {:?}", url, err))
        .map_err(|_err| StatusCode::EXPECTATION_FAILED)?;

    let fd: OwnedFd = storage
        .open_with(
            path,
            OpenOptions::new().create(true).truncate(true).write(true),
        )
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory => debug!("Failed to write file by url (open): {:?}", err),
            _ => error!("Failed to write file by url (open): {:?}", err),
        })
        .map_err(err)?
        .into_std()
        .into();
    let mut writer = BufWriter::new(tokio::fs::File::from_std(std::fs::File::from(fd)));
    while let Some(chunk) = response
        .chunk()
        .await
        .inspect_err(|err| warn!("Failed to read by url: {:?}", err))
        .map_err(|_err| StatusCode::EXPECTATION_FAILED)?
    {
        writer
            .write(&chunk)
            .await
            .inspect_err(|err| match err.kind() {
                ErrorKind::NotFound
                | ErrorKind::PermissionDenied
                | ErrorKind::IsADirectory
                | ErrorKind::NotADirectory => debug!("Failed to write file by url: {:?}", err),
                _ => error!("Failed to write file by url: {:?}", err),
            })
            .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    }
    writer
        .flush()
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;

    drop(lock);
    Ok(())
}

pub(crate) async fn mkdir_file(
    state: &DaemonState,
    id: StorageIdentifier,
    path: String,
    recursive: bool,
) -> Result<(), StatusCode> {
    let StorageLockData { lock, storage, .. } =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;
    tokio::task::spawn_blocking(move || {
        if recursive {
            storage.create_dir_all(&path)
        } else {
            storage.create_dir(&path)
        }
    })
    .await
    .inspect_err(|err| error!("Failed to mkdir (task): {:?}", err))
    .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?
    .inspect_err(|err| match err.kind() {
        ErrorKind::NotFound
        | ErrorKind::PermissionDenied
        | ErrorKind::IsADirectory
        | ErrorKind::NotADirectory
        | ErrorKind::AlreadyExists => debug!("Failed to mkdir: {:?}", err),
        _ => error!("Failed to mkdir: {:?}", err),
    })
    .map_err(err)?;
    drop(lock);
    Ok(())
}

pub(crate) async fn symlink_file(
    state: &DaemonState,
    id: StorageIdentifier,
    path: String,
    content: String,
) -> Result<(), StatusCode> {
    let StorageLockData { lock, storage, .. } =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;
    storage
        .symlink(content, path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory
            | ErrorKind::AlreadyExists => debug!("Failed to create symlink: {:?}", err),
            _ => error!("Failed to create symlink: {:?}", err),
        })
        .map_err(err)?;
    drop(lock);
    Ok(())
}

pub(crate) async fn delete_file(
    state: &DaemonState,
    id: StorageIdentifier,
    path: String,
    recursive: bool,
) -> Result<(), StatusCode> {
    let StorageLockData { lock, storage, .. } =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;
    if recursive {
        storage.remove_dir_all(&path).await
    } else {
        storage.remove_file(&path).await
    }
    .inspect_err(|err| match err.kind() {
        ErrorKind::NotFound
        | ErrorKind::PermissionDenied
        | ErrorKind::IsADirectory
        | ErrorKind::NotADirectory => debug!("Failed to delete file: {:?}", err),
        _ => error!("Failed to delete file: {:?}", err),
    })
    .map_err(err)?;
    drop(lock);
    Ok(())
}

pub(crate) async fn unpack_archive(
    state: &DaemonState,
    id: StorageIdentifier,
    path: String,
    dest: String,
) -> Result<(), StatusCode> {
    let StorageLockData { lock, storage, .. } =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let path = path_relative(path);
    let (storage, storage_path) = storage_path(id, &storage).await?;
    if !unpack_archive0(storage, storage_path, path, dest)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory
            | ErrorKind::InvalidData => debug!("Failed to unpack archive: {:?}", err),
            _ => error!("Failed to unpack archive: {:?}", err),
        })
        .map_err(|error| {
            if error.kind() == ErrorKind::InvalidData {
                return StatusCode::PRECONDITION_FAILED;
            }
            err(error)
        })?
    {
        debug!("Failed to unpack archive: unknown archive format/compression");
        return Err(StatusCode::PRECONDITION_FAILED);
    }
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn unpack_archive0(
    storage: Dir,
    storage_path: PathBuf,
    path: PathBuf,
    dest: String,
) -> Result<bool, io::Error> {
    let file = storage.open(path).await?;

    let fd: OwnedFd = file.into_std().into();
    let mut file = std::fs::File::from(fd);
    match tokio::task::spawn_blocking(move || {
        Ok::<_, io::Error>((ArchiveCompression::detect_blocking(&mut file)?, file))
    })
    .await?
    {
        Ok((Some(compression), file)) => {
            let archive = Archive::new(compression.create_decoder(file)?);
            archive.unpack_sandboxed(storage_path, dest, ()).await?;

            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(err) if err.kind() == ErrorKind::UnexpectedEof => Ok(false),
        Err(err) => Err(err),
    }
}

pub(crate) async fn create_archive(
    state: &DaemonState,
    id: StorageIdentifier,
    path: String,
    src: String,
    compression: String,
) -> Result<(), StatusCode> {
    let StorageLockData { lock, storage, .. } =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let path = path_relative(path);

    let compression: ArchiveCompression = serde_json::from_value(json!({
        "type": compression
    }))
    .map_err(|_err| StatusCode::BAD_REQUEST)?;

    let (storage, storage_path) = storage_path(id, &storage).await?;
    create_archive0(storage, storage_path, path, src, compression)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory => debug!("Failed to create archive: {:?}", err),
            _ => error!("Failed to create archive: {:?}", err),
        })
        .map_err(err)?;
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn create_archive0(
    storage: Dir,
    storage_path: PathBuf,
    path: PathBuf,
    src: String,
    compression: ArchiveCompression,
) -> Result<(), io::Error> {
    let file = storage
        .open_with(
            path,
            OpenOptions::new().create(true).truncate(true).write(true),
        )
        .await?;

    let fd: OwnedFd = file.into_std().into();
    let file = std::fs::File::from(fd);
    let archive = Builder::new(compression.create_encoder(file)?);
    let (archive, _) = archive
        .append_dir_all_sandboxed(storage_path, "", src, HashSet::new(), (), true)
        .await?;

    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        archive.into_inner().unwrap().finish()?.flush()
    })
    .await??;
    Ok(())
}

async fn storage_path(
    id: StorageIdentifier,
    storage: &StorageEntry,
) -> Result<(Dir, PathBuf), StatusCode> {
    let base = PathBuf::from(storage.path()).join(id.as_pathbuf());
    Ok((
        Dir::open_ambient_dir(&base, ambient_authority())
            .await
            .inspect_err(|err| error!("Failed to open storage {:?}: {:?}", id, err))
            .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?,
        base,
    ))
}

pub trait IntoFileResponse {
    fn into_fileresponse(
        self,
        filename: String,
        relative_path: String,
        path: Option<String>,
        archive: bool,
    ) -> FileResponse;
}

impl IntoFileResponse for Metadata {
    fn into_fileresponse(
        self,
        filename: String,
        relative_path: String,
        path: Option<String>,
        archive: bool,
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
            archive,
        }
    }
}
