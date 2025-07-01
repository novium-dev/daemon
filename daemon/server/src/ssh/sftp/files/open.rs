use crate::ssh::sftp::files::ResolvedPath;
use crate::ssh::sftp::{DaemonSftpSession, FileHandle, internal_directory_attributes, sftp_err};
use cap_async_std::fs::*;
use daemon_common::node::BackendAuthServerPermissions;
use indexmap::{IndexMap, IndexSet};
use russh_sftp::protocol::*;
use std::io::ErrorKind;
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error};
use uuid::Uuid;

pub async fn open(
    session: &mut DaemonSftpSession,
    path: String,
    options: &OpenOptions,
) -> Result<String, StatusCode> {
    open_handle(session, path, options)
        .await
        .map(|(name, handle)| {
            session.file_handles.insert(name.clone(), handle);
            name
        })
}

pub async fn open_handle(
    session: &DaemonSftpSession,
    path: String,
    options: &OpenOptions,
) -> Result<(String, FileHandle), StatusCode> {
    let ResolvedPath {
        storage,
        temp_prefix,
        temp_suffix,
    } = session.resolve_path(path)?;
    match storage {
        None => open_internal(session, &temp_prefix),
        Some((dir, permissions)) => open_regular(dir, options, permissions, &temp_suffix).await,
    }
}

#[inline(always)]
fn open_internal(
    session: &DaemonSftpSession,
    path: &[String],
) -> Result<(String, FileHandle), StatusCode> {
    let mut entries = IndexSet::new();
    for entry in session.access.keys() {
        let Some(cmp) = entry.get(0..path.len()) else {
            continue;
        };
        if *cmp == *path
            && let Some(entry) = entry.get(path.len())
        {
            entries.insert(entry.clone());
        }
    }
    if entries.is_empty() {
        if path.is_empty() {
            Ok(("empty".to_owned(), FileHandle::Empty))
        } else {
            debug!("Tried to open non-existent virtual path: {path:?}");
            Err(StatusCode::NoSuchFile)
        }
    } else {
        let entries: IndexMap<_, _> = entries
            .into_iter()
            .map(|name| (name, internal_directory_attributes()))
            .collect();
        Ok((
            format!("internal_{}", Uuid::new_v4()),
            FileHandle::Internal(entries, false, BackendAuthServerPermissions::all()),
        ))
    }
}

#[inline(always)]
async fn open_regular(
    storage: Arc<Dir>,
    options: &OpenOptions,
    permissions: BackendAuthServerPermissions,
    path_components: &[String],
) -> Result<(String, FileHandle), StatusCode> {
    let mut path = if path_components.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::new()
    };
    for comp in path_components {
        path = path.join(comp);
    }

    let (fd, handle) = if storage.is_dir(&path).await {
        let dir = storage
            .open_dir(&path)
            .await
            .inspect_err(|err| match err.kind() {
                ErrorKind::NotFound | ErrorKind::PermissionDenied => {
                    debug!("Failed to open regular dir (final): {err:?}");
                }
                _ => error!("Failed to open regular dir (final): {err:?}"),
            })
            .map_err(sftp_err)?;
        let fd_num = dir.as_raw_fd() as i32;
        let read_dir = dir
            .read_dir(".")
            .await
            .inspect_err(|err| error!("Failed to open regular dir (readdir): {err:?}"))
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
                    debug!("Failed to open regular file (final): {err:?}");
                }
                _ => error!("Failed to open regular file (final): {err:?}"),
            })
            .map_err(sftp_err)?;
        let fd: OwnedFd = file.into_std().into();
        let fd_num = fd.as_raw_fd() as i32;
        let file = tokio::fs::File::from_std(std::fs::File::from(fd));
        (fd_num, FileHandle::RegularFile(file, permissions))
    };
    Ok((fd.to_string(), handle))
}
