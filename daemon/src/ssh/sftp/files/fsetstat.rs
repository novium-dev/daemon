use crate::ssh::sftp::FileHandle;
use russh_sftp::protocol::StatusCode;
use std::fs::Permissions;
use std::io;
use std::os::unix::fs::PermissionsExt;

pub(crate) async fn fsetstat(
    handle: &mut FileHandle,
    permissions: Option<u32>,
    size: Option<u64>,
) -> Result<(), StatusCode> {
    match handle {
        FileHandle::RegularFile(file, _) => fsetstat_regular(file, permissions, size).await,
        _ => return Err(StatusCode::Failure),
    }
    .map_err(|_err| StatusCode::Failure)
}

#[inline(always)]
async fn fsetstat_regular(
    file: &mut tokio::fs::File,
    permissions: Option<u32>,
    size: Option<u64>,
) -> Result<(), io::Error> {
    if let Some(mode) = permissions {
        file.set_permissions(Permissions::from_mode(mode)).await?;
    }

    if let Some(len) = size {
        file.set_len(len).await?;
    }
    Ok(())
}
