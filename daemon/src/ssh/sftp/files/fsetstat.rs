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
use crate::ssh::sftp::{FileHandle, sftp_err};
use russh_sftp::protocol::StatusCode;
use std::fs::Permissions;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use tokio::sync::broadcast::Sender;
use tracing::{error, warn};

pub async fn fsetstat(
    handle: &mut FileHandle,
    permissions: Option<u32>,
    size: Option<u64>,
    notify: Option<Sender<StorageNotification>>,
) -> Result<(), StatusCode> {
    match handle {
        FileHandle::RegularFile(file, _) => fsetstat_regular(file, permissions, size, notify).await,
        FileHandle::RegularDir(dir, _, _, _) => {
            let file: OwnedFd = dir
                .open(".")
                .await
                .inspect_err(|err| error!("Failed fsetstat on directory open: {:?}", err))
                .map_err(|_err| StatusCode::Failure)?
                .into_std()
                .into();
            fsetstat_regular(
                &tokio::fs::File::from_std(std::fs::File::from(file)),
                permissions,
                size,
                notify,
            )
            .await
        }
        FileHandle::Internal(..) | FileHandle::Empty => Ok(()),
    }
    .inspect_err(|err| error!("Failed to fsetstat: {:?}", err))
    .map_err(sftp_err)
}

async fn fsetstat_regular(
    file: &tokio::fs::File,
    permissions: Option<u32>,
    size: Option<u64>,
    notify: Option<Sender<StorageNotification>>,
) -> Result<(), io::Error> {
    if let Some(mode) = permissions {
        file.set_permissions(Permissions::from_mode(mode)).await?;
    }

    if let Some(len) = size {
        file.set_len(len).await?;
    }

    if let Some(notify) = notify
        && let Ok(inode) = file
            .metadata()
            .await
            .map(|meta| meta.ino())
            .inspect_err(|err| {
                warn!(
                    "Fsetstat was successful, but unable to trigger storage notification: {err:?}"
                );
            })
    {
        let _ = notify
            .send(StorageNotification::Update {
                parent_inode: None,
                inode: Some(inode),
            })
            .inspect_err(|err| {
                warn!(
                    "Fsetstat was successful, but unable to trigger storage notification: {err:?}"
                );
            });
    }

    Ok(())
}
