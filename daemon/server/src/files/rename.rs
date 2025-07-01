use crate::Daemon;
use crate::files::storage_path;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::utils::path_relative;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use std::io;
use std::io::ErrorKind;
use tracing::{debug, error};

/// Cancel safe
#[inline(always)]
pub async fn rename_file<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure,
>(
    daemon: &Daemon,
    id: StorageIdentifier,
    from: String,
    to: String,
) -> Result<(), E> {
    let StorageLockData { lock, storage, .. } =
        storage_read_timeout::<E>(&daemon.config, &*daemon.conn, &daemon.storages, id).await?;
    let from = path_relative(from);
    let to = path_relative(to);
    let (storage, _) = storage_path(id, &storage).await?;
    storage
        .rename(from, &storage, to)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::AlreadyExists
            | ErrorKind::QuotaExceeded => debug!("Failed to rename file: {err:?}"),
            _ => error!("Failed to rename file: {err:?}"),
        })?;
    drop(lock);
    Ok(())
}
