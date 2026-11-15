// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::{BackupEntry, StorageEntry};
use crate::storage::StorageIdentifier;
use crate::storage::backup::BackupMetadataExt as _;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::sync::task::TaskOwnedRwLockReadGuard;
use crate::{Daemon, DaemonExtra};
use chrono::Utc;
use daemon_common::service::backup::error_backup_storage_incompatible::FromBackupStorageIncompatible;
use daemon_common::service::backup::error_backup_storage_location_not_found::FromBackupStorageLocationNotFound;
use daemon_common::service::backup::error_backup_storage_no_suitable_found::FromBackupStorageNoSuitableFound;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::task::{BackupLocation, StorageTask};
use std::collections::HashSet;
use std::io;
use std::path::Path;
use tracing::{debug, error};

/// Cancel safe
pub async fn create_backup_order<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromBackupStorageNoSuitableFound,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    name: Option<String>,
) -> Result<i64, E> {
    let StorageLockData {
        path,
        lock,
        backup_exclude,
        storage,
        ..
    } = storage_read_timeout::<E>(
        &daemon.config,
        &*daemon.conn,
        &daemon.storages,
        id,
        StorageTask::BackupCreate {
            location: None,
            progress: None,
        },
    )
    .await?;

    let mut storages: Vec<(String, i8, &BackupEntry)> = daemon
        .config
        .backup
        .iter()
        .map(|(name, entry)| (name.clone(), entry.order, entry))
        .filter_map(|(name, order, entry)| order.map(|order| (name, order, entry)))
        .collect();
    storages.sort_by_key(|storage| storage.1);
    let mut suitable = None;
    for (storage_name, _order, entry) in storages {
        if entry.backend.compatible(&storage) {
            suitable = Some((storage_name, entry));
            break;
        }
    }
    match suitable {
        None => Err(<E as FromBackupStorageNoSuitableFound>::from()),
        Some((backup_storage, backup_entry)) => {
            let (id, lock) = create_backup0::<E>(
                path,
                id,
                backup_storage,
                backup_entry,
                &storage,
                lock,
                backup_exclude,
                name,
            )
            .await?;
            drop(lock); // Ensure lock hasn't been unlocked until now
            Ok(id)
        }
    }
}

/// Cancel safe
pub async fn create_backup<
    E: From<std::io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromBackupStorageIncompatible
        + FromBackupStorageLocationNotFound,
>(
    state: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    backup_storage: &String,
    name: Option<String>,
) -> Result<i64, E> {
    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or_else(<E as FromBackupStorageLocationNotFound>::from)
        .inspect_err(|_err| debug!("Backup storage does not exist: {backup_storage}"))?;
    let StorageLockData {
        path,
        lock,
        backup_exclude,
        storage,
        ..
    } = storage_read_timeout::<E>(
        &state.config,
        &*state.conn,
        &state.storages,
        id,
        StorageTask::BackupCreate {
            location: None,
            progress: None,
        },
    )
    .await?;

    if !backup.backend.compatible(&storage) {
        return Err(<E as FromBackupStorageIncompatible>::from());
    }

    let (id, lock) = create_backup0::<E>(
        path,
        id,
        backup_storage.clone(),
        backup,
        &storage,
        lock,
        backup_exclude,
        name,
    )
    .await?;
    drop(lock);
    Ok(id)
}

/// Cancel safe
#[expect(clippy::too_many_arguments)]
async fn create_backup0<E: From<std::io::Error>>(
    path: String,
    id: StorageIdentifier,
    storage_name: String,
    backup: &BackupEntry,
    storage: &StorageEntry,
    mut lock: TaskOwnedRwLockReadGuard<StorageTask, ()>,
    mut exclude: HashSet<String>,
    name: Option<String>,
) -> Result<(i64, TaskOwnedRwLockReadGuard<StorageTask, ()>), E> {
    exclude.insert(".tmp/".to_owned());

    let timestamp = Utc::now().timestamp_millis();

    lock.task_mut(|task| {
        let StorageTask::BackupCreate { location, .. } = task else {
            unreachable!();
        };
        *location = Some(BackupLocation {
            storage: storage_name,
            id: timestamp,
        });
    });

    lock = backup
        .backend
        .create(
            id,
            Path::new(&backup.path),
            timestamp,
            storage,
            Path::new(&path),
            lock,
            exclude,
        )
        .await?;

    if let Some(name) = name.filter(|name| !name.is_empty()) {
        tokio::fs::File::open(
            &backup
                .backend
                .target_path(id, Path::new(&backup.path), timestamp),
        )
        .await
        .inspect_err(|err| error!("Backup creation failed (open for name): {err:?}"))?
        .set_backup_name(Some(name))
        .await
        .inspect_err(|err| error!("Backup creation failed (set name): {err:?}"))?;
    }

    Ok((timestamp, lock))
}
