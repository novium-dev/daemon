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
    let StorageLockData {
        lock,
        storage,
        notify,
        tokens,
        ..
    } = storage_read_timeout::<E>(&daemon.config, &*daemon.conn, &daemon.storages, id).await?;
    let (storage, _, path) = storage_path(id, &storage, path, &tokens).await?;

    let parent_ino = storage_parent_ino(&storage, &path).await;

    let (storage, path) = tokio::task::spawn_blocking(move || {
        if recursive {
            storage.create_dir_all(&path)?;
        } else {
            storage.create_dir(&path)?;
        }
        Ok::<_, io::Error>((storage, path))
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

    let ino = storage.metadata(path).await.map(|meta| meta.ino());

    if let Ok((inode, parent_inode)) = ino
        .and_then(|ino| parent_ino.map(|parent_ino| (ino, parent_ino)))
        .inspect_err(|err| {
            warn!("Mkdir was successful, but unable to trigger storage notification: {err:?}");
        })
    {
        let _ = notify
            .send(StorageNotification::Create {
                parent_inode,
                inode,
            })
            .inspect_err(|err| {
                warn!("Mkdir was successful, but unable to trigger storage notification: {err:?}");
            });
    }

    drop(lock);
    Ok(())
}
