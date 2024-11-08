use crate::ssh::sftp::FileHandle;
use russh_sftp::protocol::StatusCode;
use std::io;
use std::io::{ErrorKind, SeekFrom};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

pub(crate) async fn read(
    handle: &mut FileHandle,
    offset: u64,
    len: u32,
) -> Result<Vec<u8>, StatusCode> {
    match handle {
        FileHandle::RegularFile(file, _) => read_regular(file, offset, len).await,
        _ => return Err(StatusCode::Failure),
    }
    .map_err(|err| match err.kind() {
        ErrorKind::UnexpectedEof => StatusCode::Eof,
        _ => StatusCode::Failure,
    })
}

#[inline(always)]
async fn read_regular(
    file: &mut tokio::fs::File,
    offset: u64,
    len: u32,
) -> Result<Vec<u8>, io::Error> {
    file.seek(SeekFrom::Start(offset)).await?;
    let mut buf = vec![0; len as usize];

    let read = file.read(&mut buf).await?;
    buf.resize(read, 0u8);
    Ok(buf)
}
