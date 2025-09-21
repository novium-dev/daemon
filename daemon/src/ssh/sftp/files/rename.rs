// SPDX-License-Identifier: AGPL-3.0-only
use crate::StorageNotification;
use crate::ssh::sftp::sftp_err;
use crate::utils::storage_parent_ino;
use cap_async_std::fs::{Dir, MetadataExt as _};
use russh_sftp::protocol::StatusCode;
use std::io::ErrorKind;
use std::os::fd::AsRawFd as _;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast::Sender;
use tracing::{debug, error, warn};

pub async fn rename(
    storage_from: Arc<Dir>,
    path_from: PathBuf,
    notify_from: Option<Sender<StorageNotification>>,
    storage_to: Arc<Dir>,
    path_to: PathBuf,
    notify_to: Option<Sender<StorageNotification>>,
) -> Result<(), StatusCode> {
    let from_parent_ino = storage_parent_ino(&storage_from, &path_from).await;
    let to_parent_ino = storage_parent_ino(&storage_to, &path_to).await;
    let ino = storage_from
        .metadata(&path_from)
        .await
        .map(|meta| meta.ino());

    storage_from
        .rename(&path_from, &storage_to, &path_to)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::AlreadyExists => debug!("Failed to rename file: {err:?}"),
            _ => error!("Failed to rename file: {err:?}"),
        })
        .map_err(sftp_err)?;

    match (
        storage_from.as_raw_fd() == storage_to.as_raw_fd(),
        notify_from,
        notify_to,
    ) {
        (true, Some(notify), _) => {
            if let Ok((from_parent_inode, to_parent_inode, inode)) = from_parent_ino
                .and_then(|from_parent_ino| {
                    to_parent_ino.map(|to_parent_ino| (from_parent_ino, to_parent_ino))
                })
                .and_then(|(from_parent_ino, to_parent_ino)| {
                    ino.map(|ino| (from_parent_ino, to_parent_ino, ino))
                })
                .inspect_err(|err| {
                    warn!(
                        "Rename was successful, but unable to trigger storage notification: {err:?}"
                    );
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
        }
        (false, notify_from, notify_to) => {
            if let Some(notify) = notify_from {
                let _ = notify.send(StorageNotification::RenameFrom {
                    parent_inode: from_parent_ino.inspect_err(|err| {
                        warn!("Rename was successful, but unable to trigger storage notification: {err:?}");
                    }).ok(),
                    inode: ino.inspect_err(|err| {
                        warn!("Rename was successful, but unable to trigger storage notification: {err:?}");
                    }).ok(),
                });
            }
            if let Some(notify) = notify_to {
                let ino = storage_to.metadata(&path_to).await.map(|meta| meta.ino());
                if let Ok((inode, parent_inode)) = ino.and_then(|ino| to_parent_ino.map(|parent_ino| (ino, parent_ino)))
                    .inspect_err(|err| {
                        warn!("Rename was successful, but unable to trigger storage notification: {err:?}");
                    }) {
                    let _ = notify.send(StorageNotification::Create {
                        parent_inode,
                        inode,
                    });
                }
            }
        }
        _ => {}
    }

    Ok(())
}
