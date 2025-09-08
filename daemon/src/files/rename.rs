/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
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
pub async fn rename_file<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    from: String,
    to: String,
) -> Result<(), E> {
    let StorageLockData {
        lock,
        storage,
        notify,
        ..
    } = storage_read_timeout::<E>(&daemon.config, &*daemon.conn, &daemon.storages, id).await?;
    let (storage_from, _, path_from) = storage_path(id, &storage, from).await?;
    let (storage_to, _, path_to) = storage_path(id, &storage, to).await?;

    let from_parent_ino = storage_parent_ino(&storage_from, &path_from).await;
    let to_parent_ino = storage_parent_ino(&storage_to, &path_to).await;
    let ino = storage_from
        .metadata(&path_from)
        .await
        .map(|meta| meta.ino());

    storage_from
        .rename(path_from, &storage_to, path_to)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::AlreadyExists
            | ErrorKind::QuotaExceeded => debug!("Failed to rename file: {err:?}"),
            _ => error!("Failed to rename file: {err:?}"),
        })?;
    if let Ok((from_parent_inode, to_parent_inode, inode)) = from_parent_ino
        .and_then(|from_parent_ino| {
            to_parent_ino.map(|to_parent_ino| (from_parent_ino, to_parent_ino))
        })
        .and_then(|(from_parent_ino, to_parent_ino)| {
            ino.map(|ino| (from_parent_ino, to_parent_ino, ino))
        })
        .inspect_err(|err| {
            warn!("Rename was successful, but unable to trigger storage notification: {err:?}");
        })
    {
        let _ = notify
            .send(StorageNotification::Rename {
                from_parent_inode,
                to_parent_inode,
                inode,
            })
            .inspect_err(|err| {
                warn!("Rename was successful, but unable to trigger storage notification: {err:?}");
            });
    }
    drop(lock);
    Ok(())
}
