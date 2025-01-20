use crate::config::{ArchiveCompression, BackupEntry, StorageEntry};
use crate::storage::StorageIdentifier;
use crate::utils::{storage_read, storage_write, StorageError};
use crate::{storage, DaemonState};
use async_trait::async_trait;
use axum::body::Body;
use axum::http;
use axum::http::{HeaderMap, StatusCode};
use chrono::{TimeZone, Utc};
use daemon_common::payload::backup::BackupResponseEntry;
use serde_json::json;
use std::collections::HashSet;
use std::io;
use std::io::ErrorKind;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncWrite, AsyncWriteExt, BufReader};
use tokio_tar::{Archive, Builder};
use tokio_util::io::ReaderStream;
use tracing::{debug, error};

pub(crate) async fn create_backup_order(
    state: &DaemonState,
    id: StorageIdentifier,
) -> Result<i64, StatusCode> {
    let (path, lock, backup_exclude, storage) =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;

    let mut storages: Vec<(String, i8, &BackupEntry)> = state
        .config
        .backup
        .iter()
        .map(|(name, entry)| (name.clone(), entry.order(), entry))
        .filter(|(_, order, _)| order.is_some())
        .map(|(name, order, entry)| (name, order.unwrap(), entry))
        .collect();
    storages.sort_by_key(|storage| storage.1);
    let mut suitable = None;
    for (_, _order, entry) in storages {
        if compatible(storage, entry) {
            suitable = Some(entry);
            break;
        }
    }
    if suitable.is_none() {
        return Err(StatusCode::EXPECTATION_FAILED);
    }

    let id = create_backup0(path, id, suitable.unwrap(), storage, backup_exclude)
        .await
        .inspect_err(|err| {
            if err.kind() == ErrorKind::InvalidInput {
                debug!("Backup creation failed: {:?}", err)
            } else {
                error!("Backup creation failed: {:?}", err)
            }
        })
        .map_err(|err| {
            if err.kind() == ErrorKind::InvalidInput {
                StatusCode::UNPROCESSABLE_ENTITY
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })?;
    drop(lock);
    Ok(id)
}

fn compatible(storage: &StorageEntry, backup: &BackupEntry) -> bool {
    match (storage, backup) {
        #[cfg(feature = "btrfs")]
        (StorageEntry::Btrfs { group: group1, .. }, BackupEntry::Btrfs { group: group2, .. })
            if group1.eq(group2) =>
        {
            true
        }
        (_, BackupEntry::Archive { .. }) => true,
        #[allow(unreachable_patterns)]
        _ => false,
    }
}

pub(crate) async fn create_backup(
    state: &DaemonState,
    id: StorageIdentifier,
    backup_storage: &String,
) -> Result<i64, StatusCode> {
    let (path, lock, backup_exclude, storage) =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;

    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or(StatusCode::NOT_FOUND)
        .inspect_err(|_err| debug!("Backup storage does not exist: {}", backup_storage))?;

    let id = create_backup0(path, id, backup, storage, backup_exclude)
        .await
        .inspect_err(|err| debug!("Backup creation failed: {:?}", err))
        .map_err(|err| {
            if err.kind() == ErrorKind::InvalidInput {
                StatusCode::UNPROCESSABLE_ENTITY
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })?;
    drop(lock);
    Ok(id)
}

#[inline(always)]
async fn create_backup0(
    path: String,
    id: StorageIdentifier,
    backup: &BackupEntry,
    #[allow(unused)] storage: &StorageEntry,
    exclude: HashSet<String>,
) -> Result<i64, io::Error> {
    let dir = PathBuf::from(path);
    let target_dir = PathBuf::from(backup.path()).join(id.as_pathbuf());
    if !tokio::fs::try_exists(&target_dir).await? {
        tokio::fs::create_dir_all(&target_dir).await?;
    }
    let timestamp = Utc::now().timestamp_millis();
    match backup {
        BackupEntry::Archive {
            ref compression, ..
        } => {
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(target_dir.join(format!("{}.tar{}", timestamp, compression.ext())))
                .await?;
            let encoder = compression.create_encoder(file);
            let mut archive = tokio_tar::Builder::new(encoder);
            archive.follow_symlinks(false);
            archive
                .append_dir_all_exclude("", &dir, exclude, false)
                .await?;
            archive.finish().await?;
            archive.into_inner().await.unwrap().shutdown().await?;
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { group, .. } => {
            use std::io::ErrorKind;

            if !matches!(storage, StorageEntry::Btrfs { group: storage_group, .. } if storage_group.eq(group))
            {
                return Err(io::Error::new(ErrorKind::InvalidInput, "Group mismatch!"));
            }

            tokio::task::spawn_blocking(move || {
                let target = target_dir.join(timestamp.to_string());
                libbtrfs::subvolume::snapshot(
                    &dir.to_string_lossy(),
                    &target.to_string_lossy(),
                    true,
                )?;
                Ok::<(), io::Error>(())
            })
            .await?
            .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
        }
    }
    Ok(timestamp)
}

pub(crate) async fn restore_backup(
    state: &DaemonState,
    id: StorageIdentifier,
    backup_id: i64,
    backup_storage: &String,
) -> Result<(), StatusCode> {
    let (path, lock, disk, storage) =
        storage_write(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;

    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or(StatusCode::NOT_FOUND)
        .inspect_err(|_err| debug!("Backup storage does not exist: {}", backup_storage))?;

    restore_backup0(id, storage, path, backup_id, backup, disk)
        .await
        .inspect_err(|err| error!("Failed to restore backup: {:?}", err))
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn restore_backup0(
    id: StorageIdentifier,
    storage: &StorageEntry,
    path: String,
    backup_id: i64,
    backup: &BackupEntry,
    disk: Option<u64>,
) -> Result<(), io::Error> {
    let dir = PathBuf::from(path);
    let target_dir = PathBuf::from(backup.path()).join(id.as_pathbuf());
    match backup {
        BackupEntry::Archive {
            ref compression, ..
        } => {
            let target = target_dir.join(format!("{}.tar{}", backup_id, compression.ext()));
            let file = File::open(target).await?;
            let decoder = compression.create_decoder(BufReader::new(file));
            let mut archive = Archive::new(decoder);
            storage::delete(storage, id).await?;
            storage::ensure_exists(id, storage, disk).await?;
            archive.unpack(&dir).await?;
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { .. } => {
            use std::io::ErrorKind;
            let target = target_dir.join(backup_id.to_string());
            tokio::task::spawn_blocking(move || {
                libbtrfs::subvolume::remove(&dir.to_string_lossy())?;
                libbtrfs::subvolume::snapshot(
                    &target.to_string_lossy(),
                    &dir.to_string_lossy(),
                    false,
                )?;
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
) -> Result<Vec<BackupResponseEntry>, StatusCode> {
    let (_path, lock, _, _) =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let vec = list_backups0(state, id)
        .await
        .inspect_err(|err| error!("Failed to list backups: {:?}", err))
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    drop(lock);
    Ok(vec)
}

#[inline(always)]
async fn list_backups0(
    state: &DaemonState,
    id: StorageIdentifier,
) -> Result<Vec<BackupResponseEntry>, io::Error> {
    let mut entries = Vec::new();
    for (name, backup_storage) in &state.config.backup {
        let target_dir = PathBuf::from(backup_storage.path()).join(id.as_pathbuf());
        match backup_storage {
            BackupEntry::Archive {
                ref compression, ..
            } => {
                let mut readdir = tokio::fs::read_dir(&target_dir).await?;
                while let Some(entry) = readdir.next_entry().await? {
                    let metadata = entry.metadata().await?;
                    if !metadata.file_type().is_file() {
                        continue;
                    }
                    let filename = entry.file_name().to_string_lossy().to_string();
                    if !filename.ends_with(&format!(".tar{}", compression.ext())) {
                        continue;
                    }
                    let entry = &filename[0..filename.len() - (4 + compression.ext().len())];
                    let entry: Result<i64, _> = entry.parse();
                    if let Ok(entry) = entry {
                        entries.push(BackupResponseEntry {
                            time: entry,
                            storage: name.clone(),
                        });
                    }
                }
            }
            #[cfg(feature = "btrfs")]
            BackupEntry::Btrfs { .. } => {
                let mut readdir = tokio::fs::read_dir(&target_dir).await?;
                while let Some(entry) = readdir.next_entry().await? {
                    let metadata = entry.metadata().await?;
                    if !metadata.file_type().is_dir() {
                        continue;
                    }
                    let entry = entry.file_name().to_string_lossy().to_string();
                    let entry: Result<i64, _> = entry.parse();
                    if let Ok(entry) = entry {
                        entries.push(BackupResponseEntry {
                            time: entry,
                            storage: name.clone(),
                        });
                    }
                }
            }
        }
    }
    Ok(entries)
}

pub(crate) async fn backup_to_stream(
    state: &DaemonState,
    id: StorageIdentifier,
    backup_id: i64,
    backup_storage: &String,
) -> Result<(HeaderMap, Body), StatusCode> {
    let (_path, lock, backup_exclude, _) =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or(StatusCode::NOT_FOUND)
        .inspect_err(|_err| debug!("Backup storage does not exist: {}", backup_storage))?;
    let body = backup_to_stream0(id, backup_id, backup, backup_exclude)
        .await
        .inspect_err(|err| error!("Failed to create a backup stream: {:?}", err))
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    drop(lock);

    Ok(body)
}

#[inline(always)]
async fn backup_to_stream0(
    id: StorageIdentifier,
    backup_id: i64,
    backup: &BackupEntry,
    #[allow(unused)] exclude: HashSet<String>,
) -> Result<(HeaderMap, Body), io::Error> {
    let target_dir = PathBuf::from(backup.path()).join(id.as_pathbuf());
    match backup {
        BackupEntry::Archive {
            ref compression, ..
        } => {
            let target = target_dir.join(format!("{}.tar{}", backup_id, compression.ext()));
            let headers = HeaderMap::from_iter([(
                http::header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"{}.tar{}\"",
                    Utc.timestamp_millis_opt(backup_id).unwrap().to_rfc3339(),
                    compression.ext()
                )
                .parse()
                .unwrap(),
            )]);
            Ok((
                headers,
                Body::from_stream(ReaderStream::new(File::open(target).await?)),
            ))
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs {
            ref jit_compression,
            ..
        } => {
            let compression = *jit_compression;
            let headers = HeaderMap::from_iter([(
                http::header::CONTENT_DISPOSITION,
                format!(
                    "attachment; filename=\"{}.tar{}\"",
                    Utc.timestamp_millis_opt(backup_id).unwrap().to_rfc3339(),
                    compression.ext()
                )
                .parse()
                .unwrap(),
            )]);
            let target = target_dir.join(backup_id.to_string());
            Ok((
                headers,
                stream_realtime1(compression, target, exclude).await?,
            ))
        }
    }
}

pub(crate) async fn stream_realtime(
    state: &DaemonState,
    id: StorageIdentifier,
    compression: String,
) -> Result<(HeaderMap, Body), StatusCode> {
    let (path, lock, backup_exclude, _) =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let compression: ArchiveCompression = serde_json::from_value(json!({
        "type": compression
    }))
    .map_err(|_err| StatusCode::BAD_REQUEST)?;

    let body = stream_realtime0(path, backup_exclude, compression)
        .await
        .inspect_err(|err| error!("Failed to create a realtime backup stream: {:?}", err))
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    drop(lock);

    let headers = HeaderMap::from_iter([(
        http::header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"realtime.tar{}\"", compression.ext())
            .parse()
            .unwrap(),
    )]);

    Ok((headers, body))
}

#[inline(always)]
async fn stream_realtime0(
    path: String,
    #[allow(unused)] exclude: HashSet<String>,
    compression: ArchiveCompression,
) -> Result<Body, io::Error> {
    stream_realtime1(compression, PathBuf::from(path), exclude).await
}

async fn stream_realtime1(
    compression: ArchiveCompression,
    target: PathBuf,
    exclude: HashSet<String>,
) -> Result<Body, io::Error> {
    use tokio::io::AsyncWriteExt;

    let (sender, recv) = tokio::io::duplex(1024 * 1024);
    tokio::task::spawn(async move {
        let encoder = compression.create_encoder(sender);
        let mut archive = Builder::new(encoder);
        archive.follow_symlinks(false);
        archive
            .append_dir_all_exclude("", &target, exclude, false)
            .await?;
        archive.finish().await?;
        archive.into_inner().await.unwrap().shutdown().await?;
        Ok::<(), io::Error>(())
    });
    let stream = ReaderStream::new(recv);
    Ok(Body::from_stream(stream))
}

pub(crate) async fn delete_backup(
    state: &DaemonState,
    id: StorageIdentifier,
    backup_id: i64,
    backup_storage: &String,
) -> Result<(), StatusCode> {
    let (_path, lock, _, _) =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or(StatusCode::NOT_FOUND)
        .inspect_err(|_err| debug!("Backup storage does not exist: {}", backup_storage))?;
    delete_backup0(id, backup_id, backup)
        .await
        .inspect_err(|err| error!("Failed to delete a backup: {:?}", err))
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn delete_backup0(
    id: StorageIdentifier,
    backup_id: i64,
    backup: &BackupEntry,
) -> Result<(), io::Error> {
    let target_dir = PathBuf::from(backup.path()).join(id.as_pathbuf());
    match backup {
        BackupEntry::Archive {
            ref compression, ..
        } => {
            let target = target_dir.join(format!("{}.tar{}", backup_id, compression.ext()));
            tokio::fs::remove_file(target).await?;
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { .. } => {
            use std::io::ErrorKind;
            return tokio::task::spawn_blocking(move || {
                let target = target_dir.join(backup_id.to_string());
                libbtrfs::subvolume::remove(&target.to_string_lossy())
                    .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                Ok(())
            })
            .await
            .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
        }
    }
    Ok(())
}

#[async_trait]
trait AppendDirAllExclude {
    async fn append_dir_all_exclude<P, Q>(
        &mut self,
        path: P,
        src_path: Q,
        exclude: HashSet<String>,
        follow: bool,
    ) -> Result<(), io::Error>
    where
        P: AsRef<Path> + Send,
        Q: AsRef<Path> + Send;
}

#[async_trait]
impl<W: AsyncWrite + Unpin + Send> AppendDirAllExclude for Builder<W> {
    async fn append_dir_all_exclude<P, Q>(
        &mut self,
        path: P,
        src_path: Q,
        exclude: HashSet<String>,
        follow: bool,
    ) -> Result<(), io::Error>
    where
        P: AsRef<Path> + Send,
        Q: AsRef<Path> + Send,
    {
        let mut stack = vec![(src_path.as_ref().to_path_buf(), true, false)];
        while let Some((src, is_dir, is_symlink)) = stack.pop() {
            let dest = path
                .as_ref()
                .join(src.strip_prefix(src_path.as_ref()).unwrap());
            let mut dest_str = dest.to_string_lossy().to_string();
            if is_dir && !dest_str.ends_with("/") {
                dest_str += "/";
            }
            if exclude.contains(&dest_str) {
                continue;
            }
            // In case of a symlink pointing to a directory, is_dir is false, but src.is_dir() will return true
            if is_dir || (is_symlink && follow && src.is_dir()) {
                let mut entries = fs::read_dir(&src).await?;
                while let Some(entry) = entries.next_entry().await.transpose() {
                    let entry = entry?;
                    let file_type = entry.file_type().await?;
                    stack.push((entry.path(), file_type.is_dir(), file_type.is_symlink()));
                }
                if dest != Path::new("") {
                    self.append_dir(&dest, src).await?;
                }
            } else if !follow && is_symlink {
                self.append_path_with_name(&src, &dest).await?;
            } else {
                self.append_file(&dest, &mut File::open(src).await?).await?;
            }
        }
        Ok(())
    }
}
