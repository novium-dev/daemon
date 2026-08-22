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
use daemon_common::service::files::error_file_not_found::FromFileNotFound;
use daemon_common::service::files::task::{FileOperationKind, StorageTask};
use std::io;
use tracing::warn;
use rustix::{io::Errno, fs::{AtFlags, chownat, Uid, Gid}};

/// Cancel safe
#[inline(always)]
pub async fn chown_file<
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
    uid: Option<u32>,
    gid: Option<u32>,
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

    tokio::task::spawn_blocking(move || {
        chownat(storage.into_std_file(), path, uid.map(Uid::from_raw), gid.map(Gid::from_raw), AtFlags::SYMLINK_NOFOLLOW)
    })
    .await.map_err(io::Error::from)?.map_err(Errno::raw_os_error).map_err(io::Error::from_raw_os_error)?;

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
