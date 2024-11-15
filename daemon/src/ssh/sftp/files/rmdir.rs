use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::Dir;
use russh_sftp::protocol::StatusCode;
use std::path::PathBuf;
use tracing::debug;

pub(crate) async fn rmdir(storage: Dir, path: PathBuf) -> Result<(), StatusCode> {
    storage
        .remove_dir(path)
        .await
        .inspect_err(|err| debug!("Failed to remove dir: {:?}", err))
        .map_err(sftp_err)
}
