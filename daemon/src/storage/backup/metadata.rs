// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::BackupEntry;
use crate::storage::StorageIdentifier;
use crate::storage::backup::BackupMetadataExt as _;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::sync::task::TaskOwnedRwLockReadGuard;
use crate::{Daemon, DaemonExtra};
use daemon_common::service::backup::error_backup_storage_location_not_found::FromBackupStorageLocationNotFound;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::task::{BackupLocation, StorageTask};
use std::io;
use std::io::ErrorKind;
use std::path::PathBuf;
use tokio::fs::File;
use tracing::{debug, error};

/// Updates the metadata a backup carries in its extended attributes.
///
/// A `None` leaves the respective attribute untouched, so that protection can
/// be changed without touching the name and the other way round. An empty
/// `name` removes the name.
///
/// Cancel safe
pub async fn update_backup<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromBackupStorageLocationNotFound,
>(
    state: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    backup_id: i64,
    backup_storage: &String,
    name: Option<String>,
    protected: Option<bool>,
) -> Result<(), E> {
    let (file, lock) = open_for_update::<E>(state, id, backup_id, backup_storage).await?;
    if let Some(name) = name {
        file.set_backup_name(Some(name).filter(|name| !name.is_empty()))
            .await
            .inspect_err(|err| error!("Failed to update a backup (name): {err:?}"))?;
    }
    if let Some(protected) = protected {
        file.set_backup_protected(protected)
            .await
            .inspect_err(|err| error!("Failed to update a backup (protected): {err:?}"))?;
    }

    drop(lock);

    Ok(())
}

/// Opens the archive (or subvolume) a backup is stored as, holding the storage
/// read lock for as long as the returned guard lives.
///
/// Cancel safe
async fn open_for_update<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromBackupStorageLocationNotFound,
>(
    state: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    backup_id: i64,
    backup_storage: &String,
) -> Result<(File, TaskOwnedRwLockReadGuard<StorageTask, ()>), E> {
    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or_else(<E as FromBackupStorageLocationNotFound>::from)
        .inspect_err(|_err| debug!("Backup storage does not exist: {backup_storage}"))?;
    let StorageLockData { lock, .. } = storage_read_timeout::<E>(
        &state.config,
        &*state.conn,
        &state.storages,
        id,
        StorageTask::BackupUpdate {
            location: BackupLocation {
                storage: backup_storage.clone(),
                id: backup_id,
            },
        },
    )
    .await?;

    let target_dir = PathBuf::from(backup.path()).join(id.as_pathbuf());
    let target = match backup {
        BackupEntry::Archive { compression, .. } => {
            target_dir.join(format!("{backup_id}.tar{}", compression.ext()))
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { .. } => target_dir.join(backup_id.to_string()),
    };
    let file = File::open(target).await.inspect_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            debug!("Failed to open a backup for an update: {err:?}");
        } else {
            error!("Failed to open a backup for an update: {err:?}");
        }
    })?;
    Ok((file, lock))
}
