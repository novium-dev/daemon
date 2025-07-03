use crate::files::storage_path;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::utils::path_relative;
use crate::{Daemon, DaemonExtra};
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use std::io;
use std::io::ErrorKind;
use tracing::{debug, error};

/// Cancel safe
#[inline(always)]
pub async fn delete_file<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    path: String,
    recursive: bool,
) -> Result<(), E> {
    let StorageLockData { lock, storage, .. } =
        storage_read_timeout::<E>(&daemon.config, &*daemon.conn, &daemon.storages, id).await?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;
    if recursive {
        tokio::task::spawn(async move { storage.remove_dir_all(&path).await })
            .await
            .map_err(io::Error::from)?
    } else {
        storage.remove_file(&path).await
    }
    .inspect_err(|err| match err.kind() {
        ErrorKind::NotFound
        | ErrorKind::PermissionDenied
        | ErrorKind::IsADirectory
        | ErrorKind::NotADirectory => debug!("Failed to delete file: {err:?}"),
        _ => error!("Failed to delete file: {err:?}"),
    })?;
    drop(lock);
    Ok(())
}
