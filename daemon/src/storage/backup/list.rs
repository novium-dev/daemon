// SPDX-License-Identifier: AGPL-3.0-only
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::{Daemon, DaemonExtra};
use daemon_common::service::backup::BackupResponseEntry;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::task::StorageTask;
use std::io;
use std::path::Path;

/// Cancel safe
pub async fn list_backups<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
) -> Result<Vec<BackupResponseEntry>, E> {
    let StorageLockData { lock, .. } = storage_read_timeout::<E>(
        &daemon.config,
        &*daemon.conn,
        &daemon.storages,
        id,
        StorageTask::BackupList,
    )
    .await?;
    let mut entries = Vec::new();
    for (storage, backup_storage) in &daemon.config.backup {
        backup_storage
            .backend
            .list(storage, id, Path::new(&backup_storage.path), &mut entries)
            .await?;
    }
    drop(lock);
    Ok(entries)
}
