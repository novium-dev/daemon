use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::Dir;
use russh_sftp::protocol::StatusCode;
use std::path::PathBuf;
use tracing::debug;

pub(crate) async fn symlink(
    storage: Dir,
    path: PathBuf,
    content: String,
) -> Result<(), StatusCode> {
    let content = PathBuf::from(content);
    storage
        .symlink(content, path)
        .await
        .inspect_err(|err| debug!("Failed to symlink: {:?}", err))
        .map_err(sftp_err)
}
