use std::io;
use std::io::ErrorKind;
use std::ops::Deref;
use tracing::{debug, error};
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use crate::Daemon;
use crate::files::storage_path;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{storage_read_timeout, StorageLockData};
use crate::utils::path_relative;

pub(crate) async fn mkdir_file<E: From<io::Error> + From<sqlx::Error> + FromStorageNotFound + FromStorageMutexLocked + FromStorageFailure>(
    state: &Daemon,
    id: StorageIdentifier,
    path: String,
    recursive: bool,
) -> Result<(), E> {
    let StorageLockData { lock, storage, .. } = storage_read_timeout::<E>(
        &state.config,
        state.conn.deref(),
        &state.storages,
        id,
    )
    .await?;
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
    .inspect_err(|err| error!("Failed to mkdir (task): {:?}", err))
    .map_err(io::Error::from)?
    .inspect_err(|err| match err.kind() {
        ErrorKind::NotFound
        | ErrorKind::PermissionDenied
        | ErrorKind::NotADirectory
        | ErrorKind::AlreadyExists
        | ErrorKind::QuotaExceeded => debug!("Failed to mkdir: {:?}", err),
        _ => error!("Failed to mkdir: {:?}", err),
    })?;
    drop(lock);
    Ok(())
}
