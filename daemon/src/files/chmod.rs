// SPDX-License-Identifier: AGPL-3.0-only
use crate::files::storage_path;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::utils::storage_parent_ino;
use crate::{Daemon, DaemonExtra, StorageNotification};
use cap_async_std::fs::{MetadataExt as _, Permissions, PermissionsExt as _};
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::error_file_not_found::FromFileNotFound;
use daemon_common::service::files::task::{FileOperationKind, StorageTask};
use std::io;
use tracing::warn;

/// Cancel safe
#[inline(always)]
pub async fn chmod_file<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromFileNotFound,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    path: String,
    mode: u32,
) -> Result<(), E> {
    let StorageLockData {
        storage,
        tokens,
        notify,
        ..
    } = storage_read_timeout::<E>(
        &daemon.config,
        &*daemon.conn,
        &daemon.storages,
        id,
        StorageTask::FileOperation {
            kind: FileOperationKind::Chmod,
        },
    )
    .await?;
    let (storage, _, path) = storage_path(id, &storage, path, &tokens).await?;
    let parent_ino = storage_parent_ino(&storage, &path).await;
    let ino = storage.symlink_metadata(&path).await.map(|meta| meta.ino());

    storage
        .set_permissions(&path, Permissions::from_mode(mode))
        .await?;

    let _ = notify
        .send(StorageNotification::Update {
            parent_inode: parent_ino
                .inspect_err(|err| {
                    warn!(
                        "Chmod was successful, but unable to trigger storage notification: {err:?}"
                    );
                })
                .ok(),
            inode: ino
                .inspect_err(|err| {
                    warn!(
                        "Chmod was successful, but unable to trigger storage notification: {err:?}"
                    );
                })
                .ok(),
        })
        .inspect_err(|err| {
            warn!("Chmod was successful, but unable to trigger storage notification: {err:?}");
        });

    Ok(())
}
