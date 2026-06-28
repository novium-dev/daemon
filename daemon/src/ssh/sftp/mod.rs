// SPDX-License-Identifier: AGPL-3.0-only
pub mod ext;
pub mod files;
pub mod r#impl;

use crate::StorageNotification;
use crate::sync::task::TaskOwnedRwLockReadGuard;
use cap_async_std::fs::*;
use daemon_common::node::BackendAuthServerPermissions;
use daemon_common::service::files::task::StorageTask;
use indexmap::IndexMap;
use russh_sftp::protocol::*;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io;
use tokio::sync::broadcast::Sender;
use tracing::*;

type DaemonAccess = (
    Arc<Dir>,
    BackendAuthServerPermissions,
    TaskOwnedRwLockReadGuard<StorageTask, ()>,
    Option<Sender<StorageNotification>>,
);

pub struct DaemonSftpSession {
    pub version: Option<u32>,
    pub file_handles: HashMap<String, (FileHandle, Option<Sender<StorageNotification>>)>,
    pub access: HashMap<Vec<String>, DaemonAccess>,
}

impl DaemonSftpSession {
    fn handle(
        &mut self,
        handle: &String,
        permissions: BackendAuthServerPermissions,
    ) -> Result<(&mut FileHandle, Option<Sender<StorageNotification>>), StatusCode> {
        match self.file_handles.get_mut(handle) {
            None => {
                debug!("File handle does not exist: {handle:?}");
                Err(StatusCode::NoSuchFile)
            }
            Some((handle, sender)) => {
                if !handle.has_permissions(permissions) {
                    debug!("File handle was opened with less permissions than required");
                    return Err(StatusCode::PermissionDenied);
                }
                Ok((handle, sender.clone()))
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
