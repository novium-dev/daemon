// SPDX-License-Identifier: AGPL-3.0-only
use crate::ssh::sftp::FileHandle;
use russh_sftp::protocol::StatusCode;
use std::os::fd::{AsRawFd as _, FromRawFd as _};
use std::os::unix::fs::FileExt as _;
use std::{io, mem::ManuallyDrop};
use tracing::{debug, error};

pub async fn write(handle: &mut FileHandle, offset: u64, data: Vec<u8>) -> Result<(), StatusCode> {
    match handle {
        FileHandle::RegularFile(file, _) => write_regular(file, offset, data).await,
        _ => {
            debug!("write on a not regular file handle: {:?}", handle);
            return Err(StatusCode::OpUnsupported);
        }
    }
    .inspect_err(|err| error!("Failed to write file: {:?}", err))
    .map_err(|_err| StatusCode::Failure)
}

#[inline(always)]
async fn write_regular(
    file: &mut tokio::fs::File,
    offset: u64,
    data: Vec<u8>,
) -> Result<(), io::Error> {
    // SAFETY: The fd is open. It is not owned, but we won't close it.
    let std = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(file.as_raw_fd()) });

    tokio::task::spawn_blocking(move || std.write_all_at(&data, offset)).await??;
    Ok(())
}
