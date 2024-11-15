use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::Dir;
use russh_sftp::protocol::StatusCode;
use std::path::PathBuf;
use tracing::debug;

pub(crate) async fn remove(storage: Dir, path: PathBuf) -> Result<(), StatusCode> {
    storage
        .remove_file(path)
        .await
        .inspect_err(|err| debug!("Failed to remove file: {:?}", err))
        .map_err(sftp_err)
}
