// SPDX-License-Identifier: AGPL-3.0-only
use crate::ssh::sftp::FileHandle;
use russh_sftp::protocol::StatusCode;
use std::io;
use std::io::ErrorKind;
use std::mem::ManuallyDrop;
use std::os::fd::{AsRawFd as _, FromRawFd as _};
use std::os::unix::fs::FileExt as _;
use tracing::{debug, error};

pub async fn read(handle: &mut FileHandle, offset: u64, len: u32) -> Result<Vec<u8>, StatusCode> {
    match handle {
        FileHandle::RegularFile(file, _) => read_regular(file, offset, len).await,
        _ => {
            debug!("Read on a not regular file handle: {:?}", handle);
            return Err(StatusCode::OpUnsupported);
        }
    }
    .inspect_err(|err| error!("Failed to read file: {:?}", err))
    .map_err(|err| {
        if err.kind() == ErrorKind::UnexpectedEof {
            StatusCode::Eof
        } else {
            StatusCode::Failure
        }
    })
}

#[inline(always)]
async fn read_regular(
    file: &mut tokio::fs::File,
    offset: u64,
    len: u32,
) -> Result<Vec<u8>, io::Error> {
    // SAFETY: The fd is open. It is not owned, but we won't close it.
    let std = ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(file.as_raw_fd()) });

    tokio::task::spawn_blocking(move || {
        let mut buf = vec![0; len as usize];
        let read = std.read_at(&mut buf, offset)?;
        buf.truncate(read);
        Ok(buf)
    })
    .await?
}
