// SPDX-License-Identifier: AGPL-3.0-only
use crate::StorageNotification;
use crate::ssh::sftp::FileHandle;
use russh_sftp::protocol::StatusCode;
use std::os::unix::fs::MetadataExt as _;
use tokio::sync::broadcast::Sender;
use tracing::warn;

pub async fn close(
    handle: FileHandle,
    notify: Option<Sender<StorageNotification>>,
) -> Result<(), StatusCode> {
    match handle {
        FileHandle::RegularFile(file, _) => close_regular(file, notify).await,
        FileHandle::RegularDir(_, _, _, _) | FileHandle::Internal(_, _, _) | FileHandle::Empty => {}
    }
    Ok(())
}

async fn close_regular(file: tokio::fs::File, notify: Option<Sender<StorageNotification>>) {
    // We don't notify on every write, so we do it on close
    if let Some(notify) = notify
        && let Ok(inode) = file
            .metadata()
            .await
            .map(|meta| meta.ino())
            .inspect_err(|err| {
                warn!("Close was successful, but unable to trigger storage notification: {err:?}");
            })
    {
        let _ = notify
            .send(StorageNotification::Update {
                parent_inode: None,
                inode: Some(inode),
            })
            .inspect_err(|err| {
                warn!("Close was successful, but unable to trigger storage notification: {err:?}");
            });
    }
}
