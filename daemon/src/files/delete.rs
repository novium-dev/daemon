// SPDX-License-Identifier: AGPL-3.0-only
use crate::files::storage_path;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::utils::storage_parent_ino;
use crate::{Daemon, DaemonExtra, StorageNotification};
use cap_async_std::fs::MetadataExt as _;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use std::io;
use std::io::ErrorKind;
use tracing::{debug, error, warn};

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
    let StorageLockData {
        lock,
        storage,
        notify,
        tokens,
        ..
    } = storage_read_timeout::<E>(&daemon.config, &*daemon.conn, &daemon.storages, id).await?;
    let (storage, _, path) = storage_path(id, &storage, path, &tokens).await?;

    let ino = storage.metadata(&path).await.map(|meta| meta.ino());
    let parent_ino = storage_parent_ino(&storage, &path).await;

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

    if let Ok((inode, parent_inode)) = ino
        .and_then(|ino| parent_ino.map(|parent_ino| (ino, parent_ino)))
        .inspect_err(|err| {
            warn!("Delete was successful, but unable to trigger storage notification: {err:?}");
        })
    {
        let _ = notify
            .send(StorageNotification::Delete {
                parent_inode,
                inode,
                dir: recursive,
            })
            .inspect_err(|err| {
                warn!("Delete was successful, but unable to trigger storage notification: {err:?}");
            });
    }

    drop(lock);
    Ok(())
}
