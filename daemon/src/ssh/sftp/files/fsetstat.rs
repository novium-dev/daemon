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
use crate::ssh::sftp::{FileHandle, sftp_err};
use russh_sftp::protocol::StatusCode;
use std::fs::Permissions;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt as _;
use tracing::error;

pub async fn fsetstat(
    handle: &mut FileHandle,
    permissions: Option<u32>,
    size: Option<u64>,
) -> Result<(), StatusCode> {
    match handle {
        FileHandle::RegularFile(file, _) => fsetstat_regular(file, permissions, size).await,
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
            )
            .await
        }
        FileHandle::Internal(..) | FileHandle::Empty => Ok(()),
    }
    .inspect_err(|err| error!("Failed to fsetstat: {:?}", err))
    .map_err(sftp_err)
}

#[inline(always)]
async fn fsetstat_regular(
    file: &tokio::fs::File,
    permissions: Option<u32>,
    size: Option<u64>,
) -> Result<(), io::Error> {
    if let Some(mode) = permissions {
        file.set_permissions(Permissions::from_mode(mode)).await?;
    }

    if let Some(len) = size {
        file.set_len(len).await?;
    }
    Ok(())
}
