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

pub async fn setstat(
    storage: Arc<Dir>,
    path: PathBuf,
    permissions: Option<u32>,
    size: Option<u64>,
    notify: Option<Sender<StorageNotification>>,
) -> Result<(), StatusCode> {
    let file = storage
        .open(&path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound | ErrorKind::PermissionDenied => {
                debug!("Failed to setstat (open): {err:?}");
            }
            _ => error!("Failed to setstat (open): {err:?}"),
        })
        .map_err(sftp_err)?;
    if let Some(mode) = permissions {
        file.set_permissions(Permissions::from_mode(mode))
            .await
            .inspect_err(|err| error!("Failed to setstat (permissions): {err:?}"))
            .map_err(sftp_err)?;
    }
    if let Some(len) = size {
        file.set_len(len)
            .await
            .inspect_err(|err| error!("Failed to setstat (length): {err:?}"))
            .map_err(sftp_err)?;
    }

    if let Some(notify) = notify {
        let parent_ino = storage_parent_ino(&storage, &path).await;
        let ino = file.metadata().await.map(|meta| meta.ino());

        let _ = notify
            .send(StorageNotification::Update {
                parent_inode: parent_ino
                    .inspect_err(|err| {
                    warn!(
                        "Setstat was successful, but unable to trigger storage notification: {err:?}"
                    );
                }).ok(),
                inode: ino
                    .inspect_err(|err| {
                    warn!(
                        "Setstat was successful, but unable to trigger storage notification: {err:?}"
                    );
                }).ok(),
            })
            .inspect_err(|err| {
                warn!(
                    "Setstat was successful, but unable to trigger storage notification: {err:?}"
                );
            });
    }

    Ok(())
}
