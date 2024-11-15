pub(crate) mod ext;
pub(crate) mod files;
pub(crate) mod r#impl;

use crate::config::StorageEntry;
use crate::ssh::BackendAuthServerPermissions;
use crate::storage::StorageIdentifier;
use cap_async_std::fs::*;
use indexmap::IndexMap;
use russh_sftp::protocol::*;
use std::collections::HashMap;
use std::io::ErrorKind;
use tokio::fs::File;
use tokio::io;
use tokio::sync::OwnedRwLockReadGuard;
use tracing::*;

pub(crate) struct DaemonSftpSession {
    pub(crate) version: Option<u32>,
    pub(crate) file_handles: HashMap<String, FileHandle>,
    pub(crate) access: HashMap<
        Vec<String>,
        (
            StorageIdentifier,
            StorageEntry,
            BackendAuthServerPermissions,
            OwnedRwLockReadGuard<()>,
        ),
    >,
}

impl DaemonSftpSession {
    fn handle(
        &mut self,
        handle: String,
        permissions: BackendAuthServerPermissions,
    ) -> Result<&mut FileHandle, StatusCode> {
        match self.file_handles.get_mut(&handle) {
            None => Err(StatusCode::NoSuchFile),
            Some(handle) => {
                if !handle.has_permissions(permissions) {
                    return Err(StatusCode::PermissionDenied);
                }
                Ok(handle)
            }
        }
    }
}

pub(crate) enum FileHandle {
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
}

pub(crate) fn sftp_err(err: io::Error) -> StatusCode {
    debug!("SFTP Error: {:?}", err);
    match err.kind() {
        ErrorKind::NotFound => StatusCode::NoSuchFile,
        ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

pub(crate) fn internal_directory_attributes() -> FileAttributes {
    FileAttributes {
        uid: Some(0),
        gid: Some(0),
        size: Some(0),
        user: Some("root".to_owned()),
        group: Some("root".to_owned()),
        permissions: Some(0o755 | FileMode::DIR.bits()),
        mtime: None,
        atime: None,
    }
}
