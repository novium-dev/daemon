use crate::ssh::sftp::ext::IntoFileAttributes;
use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::{Dir, Metadata};
use russh_sftp::protocol::{FileAttributes, StatusCode};
use std::path::PathBuf;
use tracing::debug;

pub(crate) async fn stat(storage: Dir, path: PathBuf) -> Result<FileAttributes, StatusCode> {
    storage
        .metadata(path)
        .await
        .inspect_err(|err| debug!("Failed to stat file: {:?}", err))
        .map_err(sftp_err)
        .map(<Metadata as IntoFileAttributes>::into_fileattributes)
}
