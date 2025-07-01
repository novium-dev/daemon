use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::{Dir, Permissions, PermissionsExt};
use russh_sftp::protocol::StatusCode;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error};

pub async fn setstat(
    storage: Arc<Dir>,
    path: PathBuf,
    permissions: Option<u32>,
    size: Option<u64>,
) -> Result<(), StatusCode> {
    let file = storage
        .open(path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound | ErrorKind::PermissionDenied => {
                debug!("Failed to setstat (open): {:?}", err)
            }
            _ => error!("Failed to setstat (open): {:?}", err),
        })
        .map_err(sftp_err)?;
    if let Some(mode) = permissions {
        file.set_permissions(Permissions::from_mode(mode))
            .await
            .inspect_err(|err| error!("Failed to setstat (permissions): {:?}", err))
            .map_err(sftp_err)?;
    }
    if let Some(len) = size {
        file.set_len(len)
            .await
            .inspect_err(|err| error!("Failed to setstat (length): {:?}", err))
            .map_err(sftp_err)?;
    }
    Ok(())
}
