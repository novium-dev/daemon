use crate::Daemon;
use crate::config::{ArchiveCompression, BackupEntry, StorageEntry};
use crate::storage::StorageIdentifier;
use crate::storage::archive::TarSandboxedAppendExtension;
use crate::storage::btrfs::new_snapshot;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use chrono::Utc;
use daemon_common::service::backup::error_backup_storage_incompatible::FromBackupStorageIncompatible;
use daemon_common::service::backup::error_backup_storage_location_not_found::FromBackupStorageLocationNotFound;
use daemon_common::service::backup::error_backup_storage_no_suitable_found::FromBackupStorageNoSuitableFound;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use std::collections::HashSet;
use std::io;
use std::io::Write;
use std::ops::Deref;
use std::path::PathBuf;
use tracing::debug;

/// Cancel safe
pub(crate) async fn create_backup_order<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromBackupStorageNoSuitableFound,
>(
    state: &Daemon,
    id: StorageIdentifier,
) -> Result<i64, E> {
    let StorageLockData {
        path,
        lock,
        backup_exclude,
        storage,
        ..
    } = storage_read_timeout::<E>(&state.config, state.conn.deref(), &state.storages, id).await?;

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
        return Err(<E as FromBackupStorageNoSuitableFound>::from());
    }

    let id = create_backup0::<E>(path, id, suitable.unwrap(), &storage, backup_exclude).await?;
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

/// Cancel safe
pub(crate) async fn create_backup<
    E: From<std::io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromBackupStorageIncompatible
        + FromBackupStorageLocationNotFound,
>(
    state: &Daemon,
    id: StorageIdentifier,
    backup_storage: &String,
) -> Result<i64, E> {
    let StorageLockData {
        path,
        lock,
        backup_exclude,
        storage,
        ..
    } = storage_read_timeout::<E>(&state.config, state.conn.deref(), &state.storages, id).await?;

    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or_else(<E as FromBackupStorageLocationNotFound>::from)
        .inspect_err(|_err| debug!("Backup storage does not exist: {}", backup_storage))?;

    if !compatible(&storage, backup) {
        return Err(<E as FromBackupStorageIncompatible>::from());
    }

    let id = create_backup0::<E>(path, id, backup, &storage, backup_exclude).await?;
    drop(lock);
    Ok(id)
}

/// Cancel safe
async fn create_backup0<E: From<std::io::Error>>(
    path: String,
    id: StorageIdentifier,
    backup: &BackupEntry,
    #[allow(unused)] storage: &StorageEntry,
    mut exclude: HashSet<String>,
) -> Result<i64, E> {
    exclude.insert(".tmp/".to_string());

    let dir = PathBuf::from(path);
    let target_dir = PathBuf::from(backup.path()).join(id.as_pathbuf());
    if !tokio::fs::try_exists(&target_dir)
        .await
        .inspect_err(|err| debug!("Backup creation failed (try_exists): {:?}", err))?
    {
        tokio::fs::create_dir_all(&target_dir)
            .await
            .inspect_err(|err| debug!("Backup creation failed (create_dir_all): {:?}", err))?;
    }
    let timestamp = Utc::now().timestamp_millis();
    match backup {
        BackupEntry::Archive { compression, .. } => {
            tokio::task::spawn(create_archive_backup(
                dir,
                target_dir,
                timestamp,
                exclude,
                *compression,
            ))
            .await
            .map_err(io::Error::from)??;
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { .. } => {
            let subvol_fd = tokio::fs::File::open(&dir)
                .await
                .inspect_err(|err| debug!("Backup creation failed (open): {:?}", err))?;

            new_snapshot(subvol_fd, &target_dir, &timestamp.to_string(), true)
                .await
                .inspect_err(|err| debug!("Backup creation failed (snapshot): {:?}", err))?;
        }
    }
    Ok(timestamp)
}

#[inline(always)]
async fn create_archive_backup(
    dir: PathBuf,
    target_dir: PathBuf,
    timestamp: i64,
    exclude: HashSet<String>,
    compression: ArchiveCompression,
) -> Result<(), io::Error> {
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(target_dir.join(format!("{}.tar{}", timestamp, compression.ext())))
        .await
        .inspect_err(|err| debug!("Backup creation failed (open): {:?}", err))?;
    let encoder = compression
        .create_encoder(file.into_std().await)
        .inspect_err(|err| debug!("Backup creation failed (create_encoder): {:?}", err))?;
    let mut archive = tar::Builder::new(encoder);
    archive.follow_symlinks(false);
    let (archive, _) = archive
        .append_dir_all_sandboxed(&dir, "", "/", exclude, (), true)
        .await
        .inspect_err(|err| debug!("Backup creation failed (append_dir_all): {:?}", err))?;
    tokio::task::spawn_blocking(move || archive.into_inner().unwrap().finish()?.flush())
        .await
        .inspect_err(|err| debug!("Backup creation failed (spawn_blocking): {:?}", err))
        .map_err(io::Error::from)?
        .inspect_err(|err| debug!("Backup creation failed (flush): {:?}", err))?;
    Ok(())
}
