use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::{Dir, Permissions, PermissionsExt};
use russh_sftp::protocol::StatusCode;
use std::path::PathBuf;
use tracing::debug;

pub(crate) async fn mkdir(
    storage: Dir,
    path: PathBuf,
    permissions: Option<u32>,
) -> Result<(), StatusCode> {
    let (storage, path) = tokio::task::spawn_blocking(move || {
        // (copy-pasted from cap-std docs) TODO: async: fix this when we fix https://github.com/bytecodealliance/cap-std/issues/51
        storage
            .create_dir_all(&path)
            .inspect_err(|err| debug!("Failed to mkdir: {:?}", err))
            .map_err(sftp_err)?;
        Ok((storage, path))
    })
    .await
    .inspect_err(|err| debug!("Failed to mkdir, task failed: {:?}", err))
    .map_err(|_err| StatusCode::Failure)??;
    if let Some(mode) = permissions {
        storage
            .set_permissions(&path, Permissions::from_mode(mode))
            .await
            .inspect_err(|err| debug!("Failed to set permissions after mkdir: {:?}", err))
            .map_err(sftp_err)?;
    }
    Ok(())
}
