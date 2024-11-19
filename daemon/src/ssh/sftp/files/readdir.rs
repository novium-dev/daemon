use crate::ssh::sftp::ext::IntoFileAttributes;
use crate::ssh::sftp::FileHandle;
use indexmap::IndexMap;
use russh_sftp::protocol::*;
use std::io::ErrorKind;
use tokio::io;
use tracing::{debug, error};

pub(crate) async fn readdir(handle: &mut FileHandle) -> Result<Vec<File>, StatusCode> {
    Ok(match handle {
        FileHandle::RegularFile(_, _) => {
            debug!("readdir on a regular file handle: {:?}", handle);
            return Err(StatusCode::OpUnsupported);
        }
        FileHandle::RegularDir(_, read_dir, read, _) => readdir_regular(read_dir, read).await,
        FileHandle::Internal(entries, read, _) => readdir_internal(entries, read).await,
    }
    .inspect_err(|err| {
        if err.kind() != ErrorKind::UnexpectedEof {
            error!("Failed to readdir: {:?}", err)
        }
    })
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
) -> Result<IndexMap<String, FileAttributes>, io::Error> {
    let has_read = *read;
    *read = true;
    let mut files = IndexMap::new();
    loop {
        match read_dir.next() {
            Some(Ok(entry)) => {
                let name = entry.file_name().to_string_lossy().into_owned();
                let metadata = entry.metadata();
                match metadata {
                    Ok(metadata) => {
                        files.insert(name, metadata.into_fileattributes());
                    }
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
    entries: &mut IndexMap<String, FileAttributes>,
    read: &mut bool,
) -> Result<IndexMap<String, FileAttributes>, io::Error> {
    if *read {
        debug!("Tried to readdir twice!");
        return Err(io::Error::new(ErrorKind::UnexpectedEof, "Already read"));
    }
    *read = true;
    Ok(entries.to_owned())
}
