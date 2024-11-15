use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::Dir;
use russh_sftp::protocol::StatusCode;
use std::path::PathBuf;
use tracing::debug;

pub(crate) async fn readlink(storage: Dir, path: PathBuf) -> Result<String, StatusCode> {
    storage
        .read_link(path)
        .await
        .inspect_err(|err| debug!("Failed to readlink: {:?}", err))
        .map_err(sftp_err)
        .map(|buf| buf.to_string_lossy().to_string())
}
