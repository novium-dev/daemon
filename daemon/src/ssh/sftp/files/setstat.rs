use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::{Dir, Permissions, PermissionsExt};
use russh_sftp::protocol::StatusCode;
use std::path::PathBuf;

pub(crate) async fn setstat(
    storage: Dir,
    path: PathBuf,
    permissions: Option<u32>,
    size: Option<u64>,
) -> Result<(), StatusCode> {
    let file = storage.open(path).await.map_err(sftp_err)?;
    if let Some(mode) = permissions {
        file.set_permissions(Permissions::from_mode(mode))
            .await
            .map_err(sftp_err)?;
    }
    if let Some(len) = size {
        file.set_len(len).await.map_err(sftp_err)?;
    }
    Ok(())
}
