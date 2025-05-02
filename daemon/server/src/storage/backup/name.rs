use crate::Daemon;
use crate::config::BackupEntry;
use crate::storage::StorageIdentifier;
use crate::storage::backup::BackupNameExtension;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use daemon_common::service::backup::error_backup_storage_location_not_found::FromBackupStorageLocationNotFound;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use std::io;
use std::io::ErrorKind;
use std::ops::Deref;
use std::path::PathBuf;
use tokio::fs::File;
use tracing::{debug, error};

pub(crate) async fn set_backup_name<
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
    name: Option<String>,
) -> Result<(), E> {
    let StorageLockData { lock, .. } =
        storage_read_timeout::<E>(&state.config, state.conn.deref(), &state.storages, id).await?;
    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or_else(<E as FromBackupStorageLocationNotFound>::from)
        .inspect_err(|_err| debug!("Backup storage does not exist: {}", backup_storage))?;

    set_backup_name0::<E>(id, backup_id, backup, name).await?;

    drop(lock);

    Ok(())
}

#[inline(always)]
async fn set_backup_name0<E: From<io::Error>>(
    id: StorageIdentifier,
    backup_id: i64,
    backup: &BackupEntry,
    name: Option<String>,
) -> Result<(), E> {
    let target_dir = PathBuf::from(backup.path()).join(id.as_pathbuf());
    let target = match backup {
        BackupEntry::Archive { compression, .. } => {
            target_dir.join(format!("{}.tar{}", backup_id, compression.ext()))
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { .. } => target_dir.join(backup_id.to_string()),
    };
    let file = File::open(target).await.inspect_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            debug!("Failed to set backup name (open): {:?}", err)
        } else {
            error!("Failed to set backup name (open): {:?}", err)
        }
    })?;
    file.set_backup_name(name)
        .await
        .inspect_err(|err| error!("Failed to set backup name (set): {:?}", err))?;
    Ok(())
}
