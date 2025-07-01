use crate::Daemon;
use crate::config::BackupEntry;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use daemon_common::service::backup::error_backup_storage_location_not_found::FromBackupStorageLocationNotFound;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use std::io;
use std::io::ErrorKind;
use std::path::PathBuf;
use tracing::{debug, error};

/// Cancel safe
pub async fn delete_backup<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromBackupStorageLocationNotFound,
>(
    state: &Daemon,
    id: StorageIdentifier,
    backup_id: i64,
    backup_storage: &String,
) -> Result<(), E> {
    let StorageLockData { lock, .. } =
        storage_read_timeout::<E>(&state.config, &*state.conn, &state.storages, id).await?;
    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or_else(<E as FromBackupStorageLocationNotFound>::from)
        .inspect_err(|_err| debug!("Backup storage does not exist: {}", backup_storage))?;

    let target_dir = PathBuf::from(backup.path()).join(id.as_pathbuf());
    match backup {
        BackupEntry::Archive { compression, .. } => {
            let target = target_dir.join(format!("{}.tar{}", backup_id, compression.ext()));
            tokio::fs::remove_file(target).await.inspect_err(|err| {
                if err.kind() == ErrorKind::NotFound {
                    debug!("Failed to delete a backup (remove_file): {:?}", err);
                } else {
                    error!("Failed to delete a backup (remove_file): {:?}", err);
                }
            })?;
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { .. } => {
            super::super::btrfs::remove_subvolume(&target_dir, &backup_id.to_string())
                .await
                .inspect_err(|err| {
                    if err.kind() == ErrorKind::NotFound {
                        debug!("Failed to delete a backup (delete_subvolume): {:?}", err)
                    } else {
                        error!("Failed to delete a backup (delete_subvolume): {:?}", err)
                    }
                })?;
        }
    }
    drop(lock); // Ensure it gets dropped here and not earlier
    Ok(())
}
