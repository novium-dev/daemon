use crate::ssh::sftp::ext::IntoFileAttributes;
use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::{Dir, Metadata};
use russh_sftp::protocol::{FileAttributes, StatusCode};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error};

pub(crate) async fn lstat(storage: Arc<Dir>, path: PathBuf) -> Result<FileAttributes, StatusCode> {
    storage
        .symlink_metadata(path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound | ErrorKind::PermissionDenied => {
                debug!("Failed to lstat: {:?}", err)
            }
            _ => error!("Failed to lstat: {:?}", err),
        })
        .map_err(sftp_err)
        .map(<Metadata as IntoFileAttributes>::into_fileattributes)
}
