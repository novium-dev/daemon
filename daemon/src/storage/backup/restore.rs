// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::{BackupEntry, StorageEntry};
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageLockData, storage_write_timeout};
use crate::sync::task::TaskOwnedRwLockWriteGuard;
use crate::{Daemon, DaemonExtra};
use core::sync::atomic::{AtomicI64, Ordering};
use daemon_common::service::backup::error_backup_storage_location_not_found::FromBackupStorageLocationNotFound;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::task::{BackupLocation, StorageTask};
use std::io;
use std::path::Path;
use std::sync::Arc;
use tracing::debug;

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
    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or_else(<E as FromBackupStorageLocationNotFound>::from)
        .inspect_err(|_err| debug!("Backup storage does not exist: {backup_storage}"))?
        .clone();

    let StorageLockData {
        path,
        lock,
        disk,
        storage,
        value: storage_value,
        ..
    } = storage_write_timeout::<E>(
        &state.config,
        &*state.conn,
        &state.storages,
        id,
        StorageTask::BackupRestore {
            location: BackupLocation {
                storage: backup_storage.clone(),
                id: backup_id,
            },
            progress: None,
        },
    )
    .await?;

    let lock = tokio::task::spawn(restore_backup0::<E>(
        id,
        storage,
        path,
        backup_id,
        backup,
        lock,
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
#[expect(clippy::too_many_arguments)]
async fn restore_backup0<E: From<io::Error>>(
    id: StorageIdentifier,
    storage: StorageEntry,
    path: String,
    backup_id: i64,
    backup: BackupEntry,
    mut lock: TaskOwnedRwLockWriteGuard<StorageTask, ()>,
    disk: Option<u64>,
    value: Arc<(AtomicI64, AtomicI64)>,
) -> Result<TaskOwnedRwLockWriteGuard<StorageTask, ()>, E> {
    backup
        .backend
        .restore(
            id,
            Path::new(&backup.path),
            backup_id,
            &storage,
            Path::new(&path),
            &mut lock,
            disk,
            value,
        )
        .await?;
    Ok(lock)
}
