// SPDX-License-Identifier: AGPL-3.0-only
use crate::StorageNotification;
use crate::ssh::sftp::sftp_err;
use crate::utils::storage_parent_ino;
use cap_async_std::fs::{Dir, MetadataExt as _, Permissions, PermissionsExt as _};
use russh_sftp::protocol::StatusCode;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast::Sender;
use tracing::{debug, error, warn};

pub async fn mkdir(
    storage: Arc<Dir>,
    path: PathBuf,
    permissions: Option<u32>,
    notify: Option<Sender<StorageNotification>>,
) -> Result<(), StatusCode> {
    let parent_ino = storage_parent_ino(&storage, &path).await;

    let (storage, path) = tokio::task::spawn_blocking(move || {
        // (copy-pasted from cap-std docs) TODO: async: fix this when we fix https://github.com/bytecodealliance/cap-std/issues/51
        storage
            .create_dir(&path)
            .inspect_err(|err| match err.kind() {
                ErrorKind::NotFound
                | ErrorKind::PermissionDenied
                | ErrorKind::AlreadyExists
                | ErrorKind::NotADirectory => debug!("Failed to mkdir: {err:?}"),
                _ => error!("Failed to mkdir: {err:?}"),
            })
            .map_err(sftp_err)?;
        Ok((storage, path))
    })
    .await
    .inspect_err(|err| error!("Failed to mkdir, task failed: {err:?}"))
    .map_err(|_err| StatusCode::Failure)??;
    if let Some(mode) = permissions {
        storage
            .set_permissions(&path, Permissions::from_mode(mode))
            .await
            .inspect_err(|err| error!("Failed to set permissions after mkdir: {err:?}"))
            .map_err(sftp_err)?;
    }

    let ino = storage.metadata(path).await.map(|meta| meta.ino());

    if let Some(notify) = notify
        && let Ok((inode, parent_inode)) = ino
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

    Ok(())
}
