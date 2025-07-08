use crate::ssh::sftp::ext::IntoFileAttributes;
use crate::ssh::sftp::{FileHandle, internal_directory_attributes, sftp_err};
use cap_async_std::fs::*;
use russh_sftp::protocol::*;
use std::fs::Metadata as StdMetadata;
use tracing::error;

pub async fn fstat(handle: &mut FileHandle) -> Result<FileAttributes, StatusCode> {
    match handle {
        FileHandle::RegularFile(file, _) => file
            .metadata()
            .await
            .map(<StdMetadata as IntoFileAttributes>::into_fileattributes),
        FileHandle::RegularDir(dir, _, _, _) => dir
            .metadata(".")
            .await
            .map(<Metadata as IntoFileAttributes>::into_fileattributes),
        FileHandle::Internal(_, _, _) | FileHandle::Empty => Ok(internal_directory_attributes()),
    }
    .inspect_err(|err| error!("Failed to fstat: {:?}", err))
    .map_err(sftp_err)
}
