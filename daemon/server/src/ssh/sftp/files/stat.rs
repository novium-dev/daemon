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
                debug!("Failed to stat file: {:?}", err)
            }
            _ => error!("Failed to stat file: {:?}", err),
        })
        .map_err(sftp_err)
        .map(<Metadata as IntoFileAttributes>::into_fileattributes)
}
