// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::BackupEntry;
use crate::storage::StorageIdentifier;
use crate::storage::backup::BackupNameExt as _;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
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

/// Cancel safe
pub async fn set_backup_name<
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
) -> Result<(), E> {
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
    match backup {
        BackupEntry::Archive { compression, .. } => {
            let target = target_dir.join(format!("{backup_id}.tar{}", compression.ext()));
            set_file_backup_name(target, name).await?;
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { .. } => {
            set_file_backup_name(target_dir.join(backup_id.to_string()), name).await?;
        }
        BackupEntry::Rustic { password, .. } => {
            let repo_path = target_dir;
            let password = password.clone();
            tokio::task::spawn_blocking(move || {
                super::rustic::set_name(&repo_path, password, backup_id, name)
            })
            .await
            .map_err(io::Error::from)?
            .inspect_err(|err| error!("Failed to set backup name (rustic): {err:?}"))?;
        }
    }

    drop(lock);

    Ok(())
}

/// Stores the backup name in the extended attribute of the backup's backing file.
///
/// Cancel safe
async fn set_file_backup_name(target: PathBuf, name: Option<String>) -> Result<(), io::Error> {
    let file = File::open(target).await.inspect_err(|err| {
        if err.kind() == ErrorKind::NotFound {
            debug!("Failed to set backup name (open): {err:?}");
        } else {
            error!("Failed to set backup name (open): {err:?}");
        }
    })?;
    file.set_backup_name(name)
        .await
        .inspect_err(|err| error!("Failed to set backup name (set): {err:?}"))?;
    Ok(())
}
