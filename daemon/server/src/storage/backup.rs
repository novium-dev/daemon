use crate::config::{BackupEntry, StorageEntry};
use crate::storage::StorageIdentifier;
use crate::storage::archive::compression::ArchiveCompression;
use crate::storage::archive::{TarSandboxedAppendExtension, TarSandboxedUnpackExtension};
use crate::storage::btrfs::{new_snapshot, remove_subvolume};
use crate::storage::sync::{StorageError, StorageLockData, storage_read, storage_write};
use crate::{Daemon, storage};
use axum::body::Body;
use axum::http;
use axum::http::{HeaderMap, StatusCode};
use chrono::{TimeZone, Utc};
use daemon_common::payload::backup::BackupResponseEntry;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::io;
use std::io::{Error, ErrorKind, Write};
use std::ops::Deref;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use tar::{Archive, Builder};
use tokio::fs::File;
use tokio_util::io::{ReaderStream, SyncIoBridge};
use tracing::{debug, error};
use xattr::FileExt;

pub(crate) async fn create_backup_order(
    state: &Daemon,
    id: StorageIdentifier,
) -> Result<i64, StatusCode> {
    let StorageLockData {
        path,
        lock,
        backup_exclude,
        storage,
        ..
    } = storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
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
        if compatible(&storage, entry) {
            suitable = Some(entry);
            break;
        }
    }
    if suitable.is_none() {
        return Err(StatusCode::EXPECTATION_FAILED);
    }

    let id = create_backup0(path, id, suitable.unwrap(), &storage, backup_exclude)
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
    state: &Daemon,
    id: StorageIdentifier,
    backup_storage: &String,
) -> Result<i64, StatusCode> {
    let StorageLockData {
        path,
        lock,
        backup_exclude,
        storage,
        ..
    } = storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
        .await
        .map_err(StorageError::into_http)?;

    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or(StatusCode::NOT_FOUND)
        .inspect_err(|_err| debug!("Backup storage does not exist: {}", backup_storage))?;

    let id = create_backup0(path, id, backup, &storage, backup_exclude)
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
    mut exclude: HashSet<String>,
) -> Result<i64, io::Error> {
    exclude.insert(".tmp/".to_string());

    let dir = PathBuf::from(path);
    let target_dir = PathBuf::from(backup.path()).join(id.as_pathbuf());
    if !tokio::fs::try_exists(&target_dir).await? {
        tokio::fs::create_dir_all(&target_dir).await?;
    }
    let timestamp = Utc::now().timestamp_millis();
    match backup {
        BackupEntry::Archive { compression, .. } => {
            let file = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(target_dir.join(format!("{}.tar{}", timestamp, compression.ext())))
                .await?;
            let encoder = compression.create_encoder(file.into_std().await)?;
            let mut archive = tar::Builder::new(encoder);
            archive.follow_symlinks(false);
            let (archive, _) = archive
                .append_dir_all_sandboxed(&dir, "", "/", exclude, (), true)
                .await?;
            tokio::task::spawn_blocking(move || archive.into_inner().unwrap().finish()?.flush())
                .await??;
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { group, .. } => {
            use std::io::ErrorKind;

            if !matches!(storage, StorageEntry::Btrfs { group: storage_group, .. } if storage_group.eq(group))
            {
                return Err(io::Error::new(ErrorKind::InvalidInput, "Group mismatch!"));
            }

            new_snapshot(&dir, &target_dir, &timestamp.to_string(), true).await?;
        }
    }
    Ok(timestamp)
}

pub(crate) async fn restore_backup(
    state: &Daemon,
    id: StorageIdentifier,
    backup_id: i64,
    backup_storage: &String,
) -> Result<(), StatusCode> {
    let StorageLockData {
        path,
        lock,
        disk,
        storage,
        value: storage_value,
        ..
    } = storage_write(&state.config, state.conn.deref(), &state.storages, id, true)
        .await
        .map_err(StorageError::into_http)?;

    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or(StatusCode::NOT_FOUND)
        .inspect_err(|_err| debug!("Backup storage does not exist: {}", backup_storage))?;

    restore_backup0(
        id,
        &storage,
        path,
        backup_id,
        backup,
        disk,
        storage_value.clone(),
    )
    .await
    .inspect_err(|err| error!("Failed to restore backup: {:?}", err))
    .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    storage_value.0.store(-1, Ordering::Relaxed);
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
    value: Arc<(AtomicI64, AtomicI64)>,
) -> Result<(), io::Error> {
    let dir = PathBuf::from(path);
    let target_dir = PathBuf::from(backup.path()).join(id.as_pathbuf());
    match backup {
        BackupEntry::Archive { compression, .. } => {
            let target = target_dir.join(format!("{}.tar{}", backup_id, compression.ext()));
            let file = File::open(target).await?;
            let decoder = compression.create_decoder(file.into_std().await)?;
            let archive = Archive::new(decoder);
            storage::delete(storage, id).await?;
            storage::ensure_exists(id, storage, value, disk).await?;
            archive.unpack_sandboxed(&dir, "/", ()).await?;
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { .. } => {
            use std::io::ErrorKind;
            let target = target_dir.join(backup_id.to_string());
            let Some(parent) = dir.parent() else {
                error!("Got path without parent: {:?}", dir.to_string_lossy());
                return Err(io::Error::new(ErrorKind::Other, "Invalid path: No parent"));
            };
            remove_subvolume(parent, dir.file_name().unwrap().to_str().unwrap()).await?;
            new_snapshot(
                &target,
                parent,
                dir.file_name().unwrap().to_str().unwrap(),
                false,
            )
            .await?;
        }
    }
    Ok(())
}

pub(crate) async fn list_backups(
    state: &Daemon,
    id: StorageIdentifier,
) -> Result<Vec<BackupResponseEntry>, StatusCode> {
    let StorageLockData { lock, .. } =
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
    state: &Daemon,
    id: StorageIdentifier,
) -> Result<Vec<BackupResponseEntry>, io::Error> {
    let mut entries = Vec::new();
    for (storage, backup_storage) in &state.config.backup {
        let target_dir = PathBuf::from(backup_storage.path()).join(id.as_pathbuf());
        match backup_storage {
            BackupEntry::Archive { compression, .. } => {
                let mut readdir = tokio::fs::read_dir(&target_dir).await?;
                while let Some(entry) = readdir.next_entry().await? {
                    let metadata = entry.metadata().await?;

                    if !metadata.file_type().is_file() {
                        continue;
                    }
                    let entry_path = entry.path();
                    let size = metadata.size();
                    let filename = entry.file_name().to_string_lossy().to_string();
                    if !filename.ends_with(&format!(".tar{}", compression.ext())) {
                        continue;
                    }
                    let entry = &filename[0..filename.len() - (4 + compression.ext().len())];
                    let entry: Result<i64, _> = entry.parse();

                    let file = File::open(entry_path).await?;
                    let name = file.get_backup_name().await?;
                    if let Ok(entry) = entry {
                        entries.push(BackupResponseEntry {
                            time: entry,
                            storage: storage.clone(),
                            size: Some(size),
                            name,
                        });
                    }
                }
            }
            #[cfg(feature = "btrfs")]
            BackupEntry::Btrfs { quota, .. } => {
                let mut readdir = tokio::fs::read_dir(&target_dir).await?;

                let mut quotas = HashMap::new();
                if *quota {
                    quotas = tokio::task::spawn_blocking(move || {
                        if let Err(err) = super::btrfs::search_quotas(
                            std::fs::File::open(target_dir),
                            &mut quotas,
                        ) {
                            error!(
                                "Unable to retrieve quotas: {:?}. Ensure quotas are enabled",
                                err
                            );
                        }
                        quotas
                    })
                    .await?;
                }
                while let Some(entry) = readdir.next_entry().await? {
                    let metadata = entry.metadata().await?;
                    if !metadata.file_type().is_dir() {
                        continue;
                    }
                    let entry_path = entry.path();
                    let entry = entry.file_name().to_string_lossy().to_string();
                    let entry: Result<i64, _> = entry.parse();

                    let file = File::open(&entry_path).await?;
                    let name = file.get_backup_name().await?;
                    let size = if *quota {
                        match tokio::task::spawn_blocking(move || {
                            super::btrfs::get_subvol_id(Ok(file))
                        })
                        .await?
                        {
                            Ok(subvol) => quotas.get(&subvol).copied(),
                            Err(err) => {
                                error!(
                                    "Unable to retrieve subvolume id: {:?} in {:?}. Ensure this backup is a subvolume and not a directory!",
                                    err,
                                    entry_path.to_string_lossy(),
                                );
                                None
                            }
                        }
                    } else {
                        None
                    };

                    if let Ok(entry) = entry {
                        entries.push(BackupResponseEntry {
                            time: entry,
                            storage: storage.clone(),
                            size,
                            name,
                        });
                    }
                }
            }
        }
    }
    Ok(entries)
}

pub(crate) async fn set_backup_name(
    state: &Daemon,
    id: StorageIdentifier,
    backup_id: i64,
    backup_storage: &String,
    name: Option<String>,
) -> Result<(), StatusCode> {
    let StorageLockData { lock, .. } =
        storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
            .await
            .map_err(StorageError::into_http)?;
    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or(StatusCode::NOT_FOUND)
        .inspect_err(|_err| debug!("Backup storage does not exist: {}", backup_storage))?;

    set_backup_name0(id, backup_id, backup, name)
        .await
        .inspect_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                debug!("Failed to set backup name: {:?}", err)
            } else {
                error!("Failed to set backup name: {:?}", err)
            }
        })
        .map_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })?;

    drop(lock);

    Ok(())
}

#[inline(always)]
async fn set_backup_name0(
    id: StorageIdentifier,
    backup_id: i64,
    backup: &BackupEntry,
    name: Option<String>,
) -> Result<(), io::Error> {
    let target_dir = PathBuf::from(backup.path()).join(id.as_pathbuf());
    let target = match backup {
        BackupEntry::Archive { compression, .. } => {
            target_dir.join(format!("{}.tar{}", backup_id, compression.ext()))
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { .. } => target_dir.join(backup_id.to_string()),
    };
    let file = File::open(target).await?;
    file.set_backup_name(name).await?;
    Ok(())
}

pub(crate) async fn backup_to_stream(
    state: &Daemon,
    id: StorageIdentifier,
    backup_id: i64,
    backup_storage: &String,
) -> Result<(HeaderMap, Body), StatusCode> {
    let StorageLockData {
        lock,
        backup_exclude,
        ..
    } = storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
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
        .inspect_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                debug!("Failed to create a backup stream: {:?}", err)
            } else {
                error!("Failed to create a backup stream: {:?}", err)
            }
        })
        .map_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })?;
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
        BackupEntry::Archive { compression, .. } => {
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
            jit_compression, ..
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
    state: &Daemon,
    id: StorageIdentifier,
    compression: String,
) -> Result<(HeaderMap, Body), StatusCode> {
    let StorageLockData {
        path,
        lock,
        backup_exclude,
        ..
    } = storage_read(&state.config, state.conn.deref(), &state.storages, id, true)
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
    let (sender, recv) = tokio::io::duplex(1024 * 1024);
    tokio::task::spawn_blocking(move || {
        let encoder = compression.create_encoder(SyncIoBridge::new(sender))?;
        let mut archive = Builder::new(encoder);
        archive.follow_symlinks(false);
        let (archive, _) =
            archive.append_dir_all_sandboxed_blocking(&target, "", "/", exclude, (), true)?;
        archive.into_inner().unwrap().finish()?.flush()?;
        Ok::<(), io::Error>(())
    });
    let stream = ReaderStream::new(recv);
    Ok(Body::from_stream(stream))
}

pub(crate) async fn delete_backup(
    state: &Daemon,
    id: StorageIdentifier,
    backup_id: i64,
    backup_storage: &String,
) -> Result<(), StatusCode> {
    let StorageLockData { lock, .. } =
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
        .inspect_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                debug!("Failed to delete a backup: {:?}", err)
            } else {
                error!("Failed to delete a backup: {:?}", err)
            }
        })
        .map_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })?;
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
        BackupEntry::Archive { compression, .. } => {
            let target = target_dir.join(format!("{}.tar{}", backup_id, compression.ext()));
            tokio::fs::remove_file(target).await?;
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { .. } => {
            remove_subvolume(&target_dir, &backup_id.to_string()).await?;
        }
    }
    Ok(())
}

trait BackupNameExtension {
    async fn get_backup_name(&self) -> Result<Option<String>, io::Error>;

    async fn set_backup_name(&self, name: Option<String>) -> Result<(), io::Error>;
}

impl<T: AsRawFd> BackupNameExtension for T {
    async fn get_backup_name(&self) -> Result<Option<String>, Error> {
        let file = RawFdContainer(self.as_raw_fd());
        tokio::task::spawn_blocking(move || {
            match file.get_xattr(crate::common::DAEMON_BACKUP_NAME_XATTR) {
                Ok(None) => Ok(None),
                Ok(Some(bytes)) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
                Err(err) if err.raw_os_error() == Some(libc::ENOTSUP) => Ok(None),
                Err(err) => Err(err),
            }
        })
        .await?
    }

    async fn set_backup_name(&self, name: Option<String>) -> Result<(), Error> {
        let file = RawFdContainer(self.as_raw_fd());
        tokio::task::spawn_blocking(move || {
            match name {
                Some(name) => {
                    file.set_xattr(crate::common::DAEMON_BACKUP_NAME_XATTR, name.as_bytes())?
                }
                None => match file.remove_xattr(crate::common::DAEMON_BACKUP_NAME_XATTR) {
                    Ok(_) => {}
                    Err(err) if err.raw_os_error() == Some(libc::ENODATA) => {}
                    Err(err) => return Err(err),
                },
            }
            Ok(())
        })
        .await?
    }
}

struct RawFdContainer(RawFd);

impl AsRawFd for RawFdContainer {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl FileExt for RawFdContainer {}
