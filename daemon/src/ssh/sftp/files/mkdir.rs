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
use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::{Dir, Permissions, PermissionsExt as _};
use russh_sftp::protocol::StatusCode;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error};

pub async fn mkdir(
    storage: Arc<Dir>,
    path: PathBuf,
    permissions: Option<u32>,
) -> Result<(), StatusCode> {
    let (storage, path) = tokio::task::spawn_blocking(move || {
        // (copy-pasted from cap-std docs) TODO: async: fix this when we fix https://github.com/bytecodealliance/cap-std/issues/51
        storage
            .create_dir_all(&path)
            .inspect_err(|err| match err.kind() {
                ErrorKind::NotFound
                | ErrorKind::PermissionDenied
                | ErrorKind::AlreadyExists
                | ErrorKind::NotADirectory => debug!("Failed to mkdir: {:?}", err),
                _ => error!("Failed to mkdir: {:?}", err),
            })
            .map_err(sftp_err)?;
        Ok((storage, path))
    })
    .await
    .inspect_err(|err| error!("Failed to mkdir, task failed: {:?}", err))
    .map_err(|_err| StatusCode::Failure)??;
    if let Some(mode) = permissions {
        storage
            .set_permissions(&path, Permissions::from_mode(mode))
            .await
            .inspect_err(|err| error!("Failed to set permissions after mkdir: {:?}", err))
            .map_err(sftp_err)?;
    }
    Ok(())
}
