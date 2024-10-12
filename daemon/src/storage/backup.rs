use std::io;
use std::ops::DerefMut;
use std::path::PathBuf;

use axum::body::Body;
use axum::http::StatusCode;
use chrono::Utc;
use tokio::fs::{File, OpenOptions};
use tokio::io::BufReader;
use tokio_tar::Archive;
use tokio_util::io::ReaderStream;

use crate::config::{BackupMethod, DaemonConfig};
use crate::storage::StorageIdentifier;
use crate::utils::{storage_read, storage_write, StorageError};
use crate::{storage, DaemonState};

pub(crate) async fn create_backup(
    state: &DaemonState,
    id: StorageIdentifier,
) -> Result<i64, StatusCode> {
    let (path, lock) = storage_read(
        &state.config,
        state.conn.lock().await.deref_mut(),
        &state.storages,
        id,
        true,
    )
    .await
    .map_err(StorageError::into_http)?;

    let id = create_backup0(&state.config, path, id)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    drop(lock);
    Ok(id)
}

#[inline(always)]
async fn create_backup0(
    config: &DaemonConfig,
    path: String,
    id: StorageIdentifier,
) -> Result<i64, io::Error> {
    let dir = PathBuf::from(path);
    let target_dir = PathBuf::from(&config.backup.path).join(id.as_pathbuf());
    if !tokio::fs::try_exists(&target_dir).await? {
        tokio::fs::create_dir_all(&target_dir).await?;
    }
    let timestamp = Utc::now().timestamp_millis();
    match config.backup.method {
        BackupMethod::Archive { ref compression } => {
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(target_dir.join(format!("{}.tar{}", timestamp, compression.ext())))
                .await?;
            let encoder = compression.create_encoder(file);
            let mut archive = tokio_tar::Builder::new(encoder);
            archive.follow_symlinks(false);
            archive.append_dir_all(".", &dir).await?;
            archive.finish().await?;
        }
        #[cfg(feature = "btrfs")]
        BackupMethod::Btrfs { .. } => {
            use btrfsutil::subvolume::SnapshotFlags;
            use btrfsutil::BtrfsUtilError;
            use std::io::ErrorKind;

            tokio::task::spawn_blocking(move || {
                let subvol = btrfsutil::subvolume::Subvolume::get(dir.as_path())?;
                let target = target_dir.join(timestamp.to_string());
                subvol.snapshot(target.as_path(), SnapshotFlags::READ_ONLY, None)?;
                Ok::<(), BtrfsUtilError>(())
            })
            .await
            .map_err(|err| io::Error::new(ErrorKind::Other, err))?
            .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
        }
    }
    Ok(timestamp)
}

pub(crate) async fn restore_backup(
    state: &DaemonState,
    id: StorageIdentifier,
    backup: i64,
) -> Result<(), StatusCode> {
    let (path, lock, disk) = storage_write(
        &state.config,
        state.conn.lock().await.deref_mut(),
        &state.storages,
        id,
        true,
    )
    .await
    .map_err(StorageError::into_http)?;

    restore_backup0(&state.config, id, path, backup, disk)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn restore_backup0(
    config: &DaemonConfig,
    id: StorageIdentifier,
    path: String,
    timestamp: i64,
    disk: Option<u64>,
) -> Result<(), io::Error> {
    let dir = PathBuf::from(path);
    let target_dir = PathBuf::from(&config.backup.path).join(id.as_pathbuf());
    match config.backup.method {
        BackupMethod::Archive { ref compression } => {
            let target = target_dir.join(format!("{}.tar{}", timestamp, compression.ext()));
            let file = File::open(target).await?;
            let decoder = compression.create_decoder(BufReader::new(file));
            let mut archive = Archive::new(decoder);
            storage::delete(config, id).await?;
            storage::ensure_exists(config, id, disk).await?;
            archive.unpack(&dir).await?;
        }
        #[cfg(feature = "btrfs")]
        BackupMethod::Btrfs { .. } => {
            use btrfsutil::subvolume::{DeleteFlags, SnapshotFlags, Subvolume};
            use std::io::ErrorKind;
            let target = target_dir.join(timestamp.to_string());
            tokio::task::spawn_blocking(move || {
                let snapshot = Subvolume::get(target.as_path())
                    .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                let storage = Subvolume::get(dir.as_path())
                    .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                storage
                    .delete(Some(DeleteFlags::RECURSIVE))
                    .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                snapshot
                    .snapshot(dir.as_path(), Some(SnapshotFlags::empty()), None)
                    .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                Ok::<(), io::Error>(())
            })
            .await
            .map_err(|err| io::Error::new(ErrorKind::Other, err))??;
        }
    }
    Ok(())
}

pub(crate) async fn list_backups(
    state: &DaemonState,
    id: StorageIdentifier,
) -> Result<Vec<i64>, StatusCode> {
    let (_path, lock) = storage_read(
        &state.config,
        state.conn.lock().await.deref_mut(),
        &state.storages,
        id,
        true,
    )
    .await
    .map_err(StorageError::into_http)?;
    let vec = list_backups0(state, id)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    drop(lock);
    Ok(vec)
}

#[inline(always)]
async fn list_backups0(state: &DaemonState, id: StorageIdentifier) -> Result<Vec<i64>, io::Error> {
    let mut entries = Vec::new();
    let target_dir = PathBuf::from(&state.config.backup.path).join(id.as_pathbuf());
    match state.config.backup.method {
        BackupMethod::Archive { ref compression } => {
            let mut readdir = tokio::fs::read_dir(&target_dir).await?;
            while let Some(entry) = readdir.next_entry().await? {
                if !entry.file_type().await?.is_file() {
                    continue;
                }
                let filename = entry.file_name().to_string_lossy().to_string();
                if !filename.ends_with(&format!(".tar{}", compression.ext())) {
                    continue;
                }
                let entry = &filename[0..filename.len() - (4 + compression.ext().len())];
                let entry: Result<i64, _> = entry.parse();
                match entry {
                    Ok(entry) => {
                        entries.push(entry);
                    }
                    Err(_) => continue,
                }
            }
        }
        #[cfg(feature = "btrfs")]
        BackupMethod::Btrfs { .. } => {
            let mut readdir = tokio::fs::read_dir(&target_dir).await?;
            while let Some(entry) = readdir.next_entry().await? {
                if !entry.file_type().await?.is_dir() {
                    continue;
                }
                let filename = entry.file_name().to_string_lossy().to_string();
                let entry: Result<i64, _> = filename.parse();
                match entry {
                    Ok(entry) => {
                        entries.push(entry);
                    }
                    Err(_) => continue,
                }
            }
        }
    }
    Ok(entries)
}

pub(crate) async fn backup_to_stream(
    state: &DaemonState,
    id: StorageIdentifier,
    backup: i64,
) -> Result<Body, StatusCode> {
    let (_path, lock) = storage_read(
        &state.config,
        state.conn.lock().await.deref_mut(),
        &state.storages,
        id,
        true,
    )
    .await
    .map_err(StorageError::into_http)?;
    let body = backup_to_stream0(&state.config, id, backup)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    drop(lock);
    Ok(body)
}

#[inline(always)]
async fn backup_to_stream0(
    config: &DaemonConfig,
    id: StorageIdentifier,
    timestamp: i64,
) -> Result<Body, io::Error> {
    let target_dir = PathBuf::from(&config.backup.path).join(id.as_pathbuf());
    match config.backup.method {
        BackupMethod::Archive { ref compression } => {
            let target = target_dir.join(format!("{}.tar{}", timestamp, compression.ext()));
            Ok(Body::from_stream(ReaderStream::new(
                File::open(target).await?,
            )))
        }
        #[cfg(feature = "btrfs")]
        BackupMethod::Btrfs {
            ref jit_compression,
        } => {
            use tokio::io::AsyncWriteExt;
            let compression = *jit_compression;
            let target = target_dir.join(timestamp.to_string());
            let (sender, recv) = tokio::io::duplex(1024 * 1024);
            tokio::task::spawn(async move {
                let encoder = compression.create_encoder(sender);
                let mut archive = tokio_tar::Builder::new(encoder);
                archive.append_dir_all("", &target).await?;
                archive.finish().await?;
                archive.into_inner().await.unwrap().shutdown().await?;
                Ok::<(), io::Error>(())
            });
            let stream = ReaderStream::new(recv);
            Ok(Body::from_stream(stream))
        }
    }
}

pub(crate) async fn delete_backup(
    state: &DaemonState,
    id: StorageIdentifier,
    backup: i64,
) -> Result<(), StatusCode> {
    let (_path, lock) = storage_read(
        &state.config,
        state.conn.lock().await.deref_mut(),
        &state.storages,
        id,
        true,
    )
    .await
    .map_err(StorageError::into_http)?;
    delete_backup0(&state.config, id, backup)
        .await
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn delete_backup0(
    config: &DaemonConfig,
    id: StorageIdentifier,
    timestamp: i64,
) -> Result<(), io::Error> {
    let target_dir = PathBuf::from(&config.backup.path).join(id.as_pathbuf());
    match config.backup.method {
        BackupMethod::Archive { ref compression } => {
            let target = target_dir.join(format!("{}.tar{}", timestamp, compression.ext()));
            tokio::fs::remove_file(target).await?;
        }
        #[cfg(feature = "btrfs")]
        BackupMethod::Btrfs { .. } => {
            use btrfsutil::subvolume::DeleteFlags;
            use std::io::ErrorKind;
            return tokio::task::spawn_blocking(move || {
                let target = target_dir.join(timestamp.to_string());
                let subvol = btrfsutil::subvolume::Subvolume::get(target.as_path())
                    .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                subvol
                    .delete(Some(DeleteFlags::RECURSIVE))
                    .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                Ok(())
            })
            .await
            .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
        }
    }
    Ok(())
}
