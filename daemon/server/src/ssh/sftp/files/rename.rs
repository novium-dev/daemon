use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::Dir;
use russh_sftp::protocol::StatusCode;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error};

pub(crate) async fn rename(
    old_storage: Arc<Dir>,
    old_path: PathBuf,
    new_storage: Arc<Dir>,
    new_path: PathBuf,
) -> Result<(), StatusCode> {
    old_storage
        .rename(old_path, &new_storage, new_path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::AlreadyExists => debug!("Failed to rename file: {:?}", err),
            _ => error!("Failed to rename file: {:?}", err),
        })
        .map_err(sftp_err)
}
