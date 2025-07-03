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
pub async fn mkdir_file<
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
    tokio::task::spawn_blocking(move || {
        if recursive {
            storage.create_dir_all(&path)
        } else {
            storage.create_dir(&path)
        }
    })
    .await
    .inspect_err(|err| error!("Failed to mkdir (task): {err:?}"))
    .map_err(io::Error::from)?
    .inspect_err(|err| match err.kind() {
        ErrorKind::NotFound
        | ErrorKind::PermissionDenied
        | ErrorKind::NotADirectory
        | ErrorKind::AlreadyExists
        | ErrorKind::QuotaExceeded => debug!("Failed to mkdir: {err:?}"),
        _ => error!("Failed to mkdir: {err:?}"),
    })?;
    drop(lock);
    Ok(())
}
