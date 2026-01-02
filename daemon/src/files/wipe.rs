use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageLockData, storage_write_timeout};
use crate::{Daemon, DaemonExtra, storage};
use core::sync::atomic::Ordering;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::task::StorageTask;
use std::io;
use tracing::error;

pub async fn wipe<
    E: From<sqlx::Error>
        + From<io::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
) -> Result<(), E> {
    let StorageLockData {
        lock,
        storage: entry,
        value: storage_value,
        ..
    } = storage_write_timeout::<E>(
        &daemon.config,
        &*daemon.conn,
        &daemon.storages,
        id,
        StorageTask::Wipe,
    )
    .await?;
    storage::delete(&entry, id)
        .await
        .inspect_err(|err| error!("Failed to wipe storage: {err:?}"))?;
    storage_value.0.store(0, Ordering::Relaxed);

    drop(lock);
    Ok(())
}
