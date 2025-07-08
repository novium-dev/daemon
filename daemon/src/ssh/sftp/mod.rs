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
pub mod ext;
pub mod files;
pub mod r#impl;

use cap_async_std::fs::*;
use daemon_common::node::BackendAuthServerPermissions;
use indexmap::IndexMap;
use russh_sftp::protocol::*;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io;
use tokio::sync::OwnedRwLockReadGuard;
use tracing::*;

pub struct DaemonSftpSession {
    pub version: Option<u32>,
    pub file_handles: HashMap<String, FileHandle>,
    pub access: HashMap<
        Vec<String>,
        (
            Arc<Dir>,
            BackendAuthServerPermissions,
            OwnedRwLockReadGuard<()>,
        ),
    >,
}

impl DaemonSftpSession {
    fn handle(
        &mut self,
        handle: &String,
        permissions: BackendAuthServerPermissions,
    ) -> Result<&mut FileHandle, StatusCode> {
        match self.file_handles.get_mut(handle) {
            None => {
                debug!("File handle does not exist: {handle:?}");
                Err(StatusCode::NoSuchFile)
            }
            Some(handle) => {
                if !handle.has_permissions(permissions) {
                    debug!("File handle was opened with less permissions than required");
                    return Err(StatusCode::PermissionDenied);
                }
                Ok(handle)
            }
        }
    }
}

#[derive(Debug)]
pub enum FileHandle {
    RegularFile(File, BackendAuthServerPermissions),
    RegularDir(
        Dir,
        cap_async_std::fs::ReadDir,
        bool,
        BackendAuthServerPermissions,
    ),
    Internal(
        IndexMap<String, FileAttributes>,
        bool,
        BackendAuthServerPermissions,
    ),
    Empty,
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "this function should be used with .map_err, if the first argument is a reference, then it wouldn't be possible"
)]
pub fn sftp_err(err: io::Error) -> StatusCode {
    match err.kind() {
        ErrorKind::NotFound => StatusCode::NoSuchFile,
        ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

pub fn internal_directory_attributes() -> FileAttributes {
    FileAttributes {
        uid: Some(u32::from(crate::common::CONTAINER_DEFAULT_UID)),
        gid: Some(u32::from(crate::common::CONTAINER_DEFAULT_GID)),
        size: Some(0),
        user: None,
        group: None,
        permissions: Some(0o755 | FileMode::DIR.bits()),
        mtime: None,
        atime: None,
    }
}
