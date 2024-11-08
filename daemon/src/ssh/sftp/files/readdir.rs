use crate::ssh::sftp::ext::IntoFileAttributes;
use crate::ssh::sftp::FileHandle;
use russh_sftp::protocol::*;
use std::io::ErrorKind;
use tokio::io;

pub(crate) async fn readdir(handle: &mut FileHandle) -> Result<Vec<File>, StatusCode> {
    Ok(match handle {
        FileHandle::RegularFile(_, _) => return Err(StatusCode::OpUnsupported),
        FileHandle::RegularDir(_, read_dir, read, _) => readdir_regular(read_dir, read).await,
        FileHandle::Internal(entries, read, _) => readdir_internal(entries, read).await,
    }
    .map_err(|err| {
        if err.kind() == ErrorKind::UnexpectedEof {
            StatusCode::Eof
        } else {
            StatusCode::Failure
        }
    })?
    .into_iter()
    .map(|(name, attrs)| File::new(name, attrs))
    .collect())
}

#[inline(always)]
async fn readdir_regular(
    read_dir: &mut cap_async_std::fs::ReadDir,
    read: &mut bool,
) -> Result<Vec<(String, FileAttributes)>, io::Error> {
    let has_read = *read;
    *read = true;
    let mut files = Vec::new();
    loop {
        match read_dir.next() {
            Some(Ok(entry)) => {
                let name = entry.file_name().to_string_lossy().into_owned();
                let metadata = entry.metadata();
                match metadata {
                    Ok(metadata) => files.push((name, metadata.into_fileattributes())),
                    Err(err) => break Err(err),
                }
            }
            Some(Err(err)) => break Err(err),
            None if has_read && files.is_empty() => {
                break Err(io::Error::new(ErrorKind::UnexpectedEof, ""))
            }
            None => break Ok(files),
        }
    }
}

#[inline(always)]
async fn readdir_internal(
    entries: &mut Vec<(String, FileAttributes)>,
    read: &mut bool,
) -> Result<Vec<(String, FileAttributes)>, io::Error> {
    if *read {
        return Err(io::Error::new(ErrorKind::UnexpectedEof, "Already read"));
    }
    *read = true;
    Ok(entries.to_owned())
}
