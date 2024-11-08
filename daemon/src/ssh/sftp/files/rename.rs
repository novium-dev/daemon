use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::Dir;
use russh_sftp::protocol::StatusCode;
use std::path::PathBuf;

pub(crate) async fn rename(
    old_storage: Dir,
    old_path: PathBuf,
    new_storage: Dir,
    new_path: PathBuf,
) -> Result<(), StatusCode> {
    old_storage
        .rename(old_path, &new_storage, new_path)
        .await
        .map_err(sftp_err)
}
