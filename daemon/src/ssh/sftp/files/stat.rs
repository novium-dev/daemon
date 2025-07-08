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
use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::{Dir, Metadata};
use russh_sftp::protocol::{FileAttributes, StatusCode};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error};

pub async fn stat(storage: Arc<Dir>, path: PathBuf) -> Result<FileAttributes, StatusCode> {
    storage
        .metadata(path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound | ErrorKind::PermissionDenied => {
                debug!("Failed to stat file: {err:?}");
            }
            _ => error!("Failed to stat file: {err:?}"),
        })
        .map_err(sftp_err)
        .map(<Metadata as IntoFileAttributes>::into_fileattributes)
}
