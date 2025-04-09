use crate::ssh::sftp::files::fstat;
use crate::ssh::sftp::{FileHandle, sftp_err};
use russh_sftp::protocol::{FileAttributes, StatusCode};
use std::fs::Permissions;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt;
use tracing::error;

pub(crate) async fn fsetstat(
    handle: &mut FileHandle,
    permissions: Option<u32>,
    size: Option<u64>,
) -> Result<FileAttributes, StatusCode> {
    match handle {
        FileHandle::RegularFile(file, _) => fsetstat_regular(file, permissions, size).await,
        FileHandle::RegularDir(dir, _, _, _) => {
            let file: OwnedFd = dir
                .open(".")
                .await
                .inspect_err(|err| error!("Failed fsetstat on directory open: {:?}", err))
                .map_err(|_err| StatusCode::Failure)?
                .into_std()
                .into();
            fsetstat_regular(
                &mut tokio::fs::File::from_std(std::fs::File::from(file)),
                permissions,
                size,
            )
            .await
        }
        _ => Ok(()),
    }
    .inspect_err(|err| error!("Failed to fsetstat: {:?}", err))
    .map_err(sftp_err)?;
    fstat(handle).await
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
