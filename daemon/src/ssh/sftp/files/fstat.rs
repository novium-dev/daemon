use crate::ssh::sftp::ext::IntoFileAttributes;
use crate::ssh::sftp::{internal_directory_attributes, sftp_err, FileHandle};
use cap_async_std::fs::*;
use russh_sftp::protocol::*;
use std::fs::Metadata as StdMetadata;
use tracing::debug;

pub(crate) async fn fstat(handle: &mut FileHandle) -> Result<FileAttributes, StatusCode> {
    match handle {
        FileHandle::RegularFile(file, _) => file
            .metadata()
            .await
            .map(<StdMetadata as IntoFileAttributes>::into_fileattributes),
        FileHandle::RegularDir(dir, _, _, _) => dir
            .metadata(".")
            .await
            .map(<Metadata as IntoFileAttributes>::into_fileattributes),
        FileHandle::Internal(_, _, _) => Ok(internal_directory_attributes()),
    }
    .inspect_err(|err| debug!("Failed to fstat: {:?}", err))
    .map_err(sftp_err)
}
