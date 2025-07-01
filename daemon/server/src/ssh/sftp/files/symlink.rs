use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::Dir;
use russh_sftp::protocol::StatusCode;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error};

pub async fn symlink(
    storage: Arc<Dir>,
    path: PathBuf,
    content: String,
) -> Result<(), StatusCode> {
    let content = PathBuf::from(content);
    storage
        .symlink(content, path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound | ErrorKind::PermissionDenied | ErrorKind::AlreadyExists => {
                debug!("Failed to symlink: {err:?}");
            }
            _ => error!("Failed to symlink: {err:?}"),
        })
        .map_err(sftp_err)
}
