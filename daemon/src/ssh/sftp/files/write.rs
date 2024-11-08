use crate::ssh::sftp::FileHandle;
use russh_sftp::protocol::StatusCode;
use std::io;
use std::io::{ErrorKind, SeekFrom};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

pub(crate) async fn write(
    handle: &mut FileHandle,
    offset: u64,
    data: Vec<u8>,
) -> Result<(), StatusCode> {
    match handle {
        FileHandle::RegularFile(file, _) => write_regular(file, offset, data).await,
        _ => return Err(StatusCode::Failure),
    }
    .map_err(|err| match err.kind() {
        ErrorKind::UnexpectedEof => StatusCode::Eof,
        _ => StatusCode::Failure,
    })
}

#[inline(always)]
async fn write_regular(
    file: &mut tokio::fs::File,
    offset: u64,
    data: Vec<u8>,
) -> Result<(), io::Error> {
    file.seek(SeekFrom::Start(offset)).await?;
    file.write_all(&data).await?;
    Ok(())
}
