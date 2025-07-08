/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
use crate::config::{BackupEntry, StorageEntry};
use crate::storage::StorageIdentifier;
use crate::storage::archive::TarSandboxedUnpackExt as _;
use crate::storage::sync::{StorageLockData, storage_write_timeout};
use crate::{Daemon, DaemonExtra, storage};
use core::sync::atomic::{AtomicI64, Ordering};
use daemon_common::service::backup::error_backup_storage_location_not_found::FromBackupStorageLocationNotFound;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use std::io;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use tar::Archive;
use tokio::fs::File;
use tracing::{debug, error};

/// Cancel safe
pub async fn restore_backup<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromBackupStorageLocationNotFound
        + Send
        + 'static,
>(
    state: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    backup_id: i64,
    backup_storage: &String,
) -> Result<(), E> {
    let StorageLockData {
        path,
        lock,
        disk,
        storage,
        value: storage_value,
        ..
    } = storage_write_timeout::<E>(&state.config, &*state.conn, &state.storages, id).await?;

    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or_else(<E as FromBackupStorageLocationNotFound>::from)
        .inspect_err(|_err| debug!("Backup storage does not exist: {}", backup_storage))?
        .clone();

    tokio::task::spawn(restore_backup0::<E>(
        id,
        storage,
        path,
        backup_id,
        backup,
        disk,
        Arc::clone(&storage_value),
    ))
    .await
    .map_err(io::Error::from)??;
    storage_value.0.store(-1, Ordering::Relaxed);
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn restore_backup0<E: From<io::Error>>(
    id: StorageIdentifier,
    storage: StorageEntry,
    path: String,
    backup_id: i64,
    backup: BackupEntry,
    disk: Option<u64>,
    value: Arc<(AtomicI64, AtomicI64)>,
) -> Result<(), E> {
    let dir = PathBuf::from(path);
    let target_dir = PathBuf::from(backup.path()).join(id.as_pathbuf());
    match backup {
        BackupEntry::Archive { compression, .. } => {
            let target = target_dir.join(format!("{backup_id}.tar{}", compression.ext()));
            let file = File::open(target).await.inspect_err(|err| {
                if err.kind() == ErrorKind::NotFound {
                    debug!("Failed to restore backup (open): {err:?}");
                } else {
                    error!("Failed to restore backup (open): {err:?}");
                }
            })?;
            let decoder = compression
                .create_decoder(file.into_std().await)
                .inspect_err(|err| {
                    error!("Failed to restore backup (create_decoder): {err:?}");
                })?;
            let archive = Archive::new(decoder);
            storage::delete(&storage, id).await.inspect_err(|err| {
                error!("Failed to restore backup (storage_delete): {err:?}");
            })?;
            storage::prepare(id, &storage, &value, disk)
                .await
                .inspect_err(|err| {
                    error!("Failed to restore backup (storage_prepare): {err:?}");
                })?;
            archive
                .unpack_sandboxed(&dir, "/", ())
                .await
                .inspect_err(|err| error!("Failed to restore backup (unpack): {err:?}"))?;
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { .. } => {
            use std::io::ErrorKind;
            let target = target_dir.join(backup_id.to_string());
            let parent = dir.join("..");
            let fd = tokio::fs::File::open(&target).await.inspect_err(|err| {
                if err.kind() == ErrorKind::NotFound {
                    debug!("Failed to restore backup (open): {err:?}");
                } else {
                    error!("Failed to restore backup (open): {err:?}");
                }
            })?;
            let Some(filename) = dir.file_name() else {
                error!("Missing final component in storage path");
                return Err(E::from(io::Error::new(
                    ErrorKind::InvalidData,
                    "No final component in storage path",
                )));
            };
            let Some(filename) = filename.to_str() else {
                error!("Final component in storage path is not utf8");
                return Err(E::from(io::Error::new(
                    ErrorKind::InvalidData,
                    "Final component in storage path is not utf8",
                )));
            };
            super::super::btrfs::remove_subvolume(&parent, filename)
                .await
                .inspect_err(|err| error!("Failed to restore backup (delete_snapshot): {err:?}"))?;
            super::super::btrfs::new_snapshot(fd, &parent, filename, false)
                .await
                .inspect_err(|err| error!("Failed to restore backup (create_snapshot): {err:?}"))?;
        }
    }
    Ok(())
}
