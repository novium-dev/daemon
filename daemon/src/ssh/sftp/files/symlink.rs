// SPDX-License-Identifier: AGPL-3.0-only
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

pub async fn symlink(
    storage: Arc<Dir>,
    path: PathBuf,
    content: String,
    notify: Option<Sender<StorageNotification>>,
) -> Result<(), StatusCode> {
    let content = PathBuf::from(content);

    let parent_ino = storage_parent_ino(&storage, &path).await;

    storage
        .symlink(content, &path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound | ErrorKind::PermissionDenied | ErrorKind::AlreadyExists => {
                debug!("Failed to symlink: {err:?}");
            }
            _ => error!("Failed to symlink: {err:?}"),
        })
        .map_err(sftp_err)?;

    let ino = storage.symlink_metadata(&path).await.map(|meta| meta.ino());

    if let Some(notify) = notify
        && let Ok((inode, parent_inode)) = ino
            .and_then(|ino| parent_ino.map(|parent_ino| (ino, parent_ino)))
            .inspect_err(|err| {
                warn!(
                    "Symlink was successful, but unable to trigger storage notification: {err:?}"
                );
            })
    {
        let _ = notify
            .send(StorageNotification::Create {
                parent_inode,
                inode,
            })
            .inspect_err(|err| {
                warn!(
                    "Symlink was successful, but unable to trigger storage notification: {err:?}"
                );
            });
    }

    Ok(())
}
