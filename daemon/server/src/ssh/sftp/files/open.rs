use crate::ssh::sftp::{DaemonSftpSession, FileHandle, internal_directory_attributes, sftp_err};
use cap_async_std::fs::*;
use daemon_common::node::BackendAuthServerPermissions;
use indexmap::{IndexMap, IndexSet};
use russh_sftp::protocol::*;
use std::io::ErrorKind;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error};
use uuid::Uuid;

pub(crate) async fn open(
    session: &mut DaemonSftpSession,
    path: String,
    options: &OpenOptions,
) -> Result<String, StatusCode> {
    let (storage, temp_prefix, temp_suffix) = session.resolve_path(path).await?;
    match storage {
        None => open_internal(session, temp_prefix).await,
        Some((dir, permissions)) => {
            open_regular(session, dir, options, permissions, temp_suffix).await
        }
    }
}

#[inline(always)]
async fn open_internal(
    session: &mut DaemonSftpSession,
    path: Vec<String>,
) -> Result<String, StatusCode> {
    let mut entries = IndexSet::new();
    for entry in session.access.keys() {
        if path.len() >= entry.len() {
            continue;
        }
        if entry[0..path.len()].eq(&path) {
            entries.insert(entry[path.len()].clone());
        }
    }
    if entries.is_empty() {
        if path.is_empty() {
            Ok("empty".to_string())
        } else {
            debug!("Tried to open non-existent virtual path: {:?}", path);
            Err(StatusCode::NoSuchFile)
        }
    } else {
        let entries: IndexMap<_, _> = entries
            .into_iter()
            .map(|name| (name, internal_directory_attributes()))
            .collect();
        let handle = format!("internal_{}", Uuid::new_v4());
        session.file_handles.insert(
            handle.clone(),
            FileHandle::Internal(entries, false, BackendAuthServerPermissions::all()),
        );
        Ok(handle)
    }
}

#[inline(always)]
async fn open_regular(
    session: &mut DaemonSftpSession,
    storage: Arc<Dir>,
    options: &OpenOptions,
    permissions: BackendAuthServerPermissions,
    path_components: Vec<String>,
) -> Result<String, StatusCode> {
    let mut path = if path_components.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::new()
    };
    for comp in &path_components {
        path = path.join(comp);
    }

    let (fd, handle) = if storage.is_dir(&path).await {
        let dir = storage
            .open_dir(&path)
            .await
            .inspect_err(|err| match err.kind() {
                ErrorKind::NotFound | ErrorKind::PermissionDenied => {
                    debug!("Failed to open regular dir (final): {:?}", err)
                }
                _ => error!("Failed to open regular dir (final): {:?}", err),
            })
            .map_err(sftp_err)?;
        let fd_num = dir.as_raw_fd() as i32;
        let read_dir = dir
            .read_dir(".")
            .await
            .inspect_err(|err| error!("Failed to open regular dir (readdir): {:?}", err))
            .map_err(sftp_err)?;
        (
            fd_num,
            FileHandle::RegularDir(dir, read_dir, false, permissions),
        )
    } else {
        let file = storage
            .open_with(&path, options)
            .await
            .inspect_err(|err| match err.kind() {
                ErrorKind::NotFound | ErrorKind::PermissionDenied => {
                    debug!("Failed to open regular file (final): {:?}", err)
                }
                _ => error!("Failed to open regular file (final): {:?}", err),
            })
            .map_err(sftp_err)?;
        let fd: OwnedFd = file.into_std().into();
        let fd_num = fd.as_raw_fd() as i32;
        let file = tokio::fs::File::from_std(std::fs::File::from(fd));
        (fd_num, FileHandle::RegularFile(file, permissions))
    };
    let handle_id = fd.to_string();
    session.file_handles.insert(handle_id.clone(), handle);
    Ok(handle_id)
}
