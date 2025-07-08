use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::Dir;
use russh_sftp::protocol::StatusCode;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error};

pub async fn rmdir(storage: Arc<Dir>, path: PathBuf) -> Result<(), StatusCode> {
    storage
        .remove_dir(path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::NotADirectory
            | ErrorKind::DirectoryNotEmpty => debug!("Failed to remove dir: {:?}", err),
            _ => error!("Failed to remove dir: {:?}", err),
        })
        .map_err(sftp_err)
}
