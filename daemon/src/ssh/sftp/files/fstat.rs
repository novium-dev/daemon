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
use crate::ssh::sftp::ext::IntoFileAttributes;
use crate::ssh::sftp::{FileHandle, internal_directory_attributes, sftp_err};
use cap_async_std::fs::*;
use russh_sftp::protocol::*;
use std::fs::Metadata as StdMetadata;
use tracing::error;

pub async fn fstat(handle: &mut FileHandle) -> Result<FileAttributes, StatusCode> {
    match handle {
        FileHandle::RegularFile(file, _) => file
            .metadata()
            .await
            .map(<StdMetadata as IntoFileAttributes>::into_fileattributes),
        FileHandle::RegularDir(dir, _, _, _) => dir
            .metadata(".")
            .await
            .map(<Metadata as IntoFileAttributes>::into_fileattributes),
        FileHandle::Internal(_, _, _) | FileHandle::Empty => Ok(internal_directory_attributes()),
    }
    .inspect_err(|err| error!("Failed to fstat: {:?}", err))
    .map_err(sftp_err)
}
