use crate::config::{BackupEntry, StorageEntry};
use crate::storage::StorageIdentifier;
use crate::storage::archive::TarSandboxedUnpackExtension;
use crate::storage::btrfs::{new_snapshot, remove_subvolume};
use crate::storage::sync::{StorageLockData, storage_write_timeout};
use crate::{Daemon, storage};
use daemon_common::service::backup::error_backup_storage_location_not_found::FromBackupStorageLocationNotFound;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use std::io;
use std::io::ErrorKind;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use tar::Archive;
use tokio::fs::File;
use tracing::{debug, error};

/// Cancel safe
pub(crate) async fn restore_backup<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromBackupStorageLocationNotFound
        + Send
        + 'static,
>(
    state: &Daemon,
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
    } = storage_write_timeout::<E>(&state.config, state.conn.deref(), &state.storages, id).await?;

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
        storage_value.clone(),
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
            let target = target_dir.join(format!("{}.tar{}", backup_id, compression.ext()));
            let file = File::open(target).await.inspect_err(|err| {
                if err.kind() == ErrorKind::NotFound {
                    debug!("Failed to restore backup (open): {:?}", err)
                } else {
                    error!("Failed to restore backup (open): {:?}", err)
                }
            })?;
            let decoder = compression
                .create_decoder(file.into_std().await)
                .inspect_err(|err| {
                    error!("Failed to restore backup (create_decoder): {:?}", err)
                })?;
            let archive = Archive::new(decoder);
            storage::delete(&storage, id).await.inspect_err(|err| {
                error!("Failed to restore backup (storage_delete): {:?}", err)
            })?;
            storage::prepare(id, &storage, value, disk)
                .await
                .inspect_err(|err| {
                    error!("Failed to restore backup (storage_prepare): {:?}", err)
                })?;
            archive
                .unpack_sandboxed(&dir, "/", ())
                .await
                .inspect_err(|err| error!("Failed to restore backup (unpack): {:?}", err))?;
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { .. } => {
            use std::io::ErrorKind;
            let target = target_dir.join(backup_id.to_string());
            let parent = dir.join("..");
            let fd = tokio::fs::File::open(&target).await.inspect_err(|err| {
                if err.kind() == ErrorKind::NotFound {
                    debug!("Failed to restore backup (open): {:?}", err)
                } else {
                    error!("Failed to restore backup (open): {:?}", err)
                }
            })?;
            remove_subvolume(&parent, dir.file_name().unwrap().to_str().unwrap())
                .await
                .inspect_err(|err| {
                    error!("Failed to restore backup (delete_snapshot): {:?}", err)
                })?;
            new_snapshot(
                fd,
                &parent,
                dir.file_name().unwrap().to_str().unwrap(),
                false,
            )
            .await
            .inspect_err(|err| error!("Failed to restore backup (create_snapshot): {:?}", err))?;
        }
    }
    Ok(())
}
