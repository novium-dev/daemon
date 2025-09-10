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
use crate::StorageNotification;
use crate::ssh::sftp::sftp_err;
use crate::utils::storage_parent_ino;
use cap_async_std::fs::{Dir, MetadataExt as _};
use russh_sftp::protocol::StatusCode;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast::Sender;
use tracing::{debug, error, warn};

pub async fn rmdir(
    storage: Arc<Dir>,
    path: PathBuf,
    notify: Option<Sender<StorageNotification>>,
) -> Result<(), StatusCode> {
    let ino = storage.metadata(&path).await.map(|meta| meta.ino());
    let parent_ino = storage_parent_ino(&storage, &path).await;

    storage
        .remove_dir(path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::NotADirectory
            | ErrorKind::DirectoryNotEmpty => debug!("Failed to remove dir: {:?}", err),
            _ => error!("Failed to remove dir: {:?}", err),
        })
        .map_err(sftp_err)?;

    if let Some(notify) = notify
        && let Ok((inode, parent_inode)) = ino
            .and_then(|ino| parent_ino.map(|parent_ino| (ino, parent_ino)))
            .inspect_err(|err| {
                warn!("Delete was successful, but unable to trigger storage notification: {err:?}");
            })
    {
        let _ = notify
            .send(StorageNotification::Delete {
                parent_inode,
                inode,
                dir: false,
            })
            .inspect_err(|err| {
                warn!("Delete was successful, but unable to trigger storage notification: {err:?}");
            });
    }

    Ok(())
}
