// Own implementation of directory handles with libc and tokio, because rust doesn't support these yet
// https://github.com/rust-lang/rust/issues/120426

mod small_c_string;

use crate::dirfd::small_c_string::run_path_with_cstr;
use libc::{c_int, DIR};
use std::collections::VecDeque;
use std::fs::{FileType, ReadDir};
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{io, mem};
use tokio::fs::{File, ReadDir as ReadDirTokio};
use tokio::task::JoinHandle;

const CHUNK_SIZE: usize = 32;

pub(crate) trait DirectoryFDExt {
    async fn opendir(path: impl AsRef<Path>) -> io::Result<File>;

    async fn readdir(&self) -> io::Result<ReadDirTokio>;
}

impl DirectoryFDExt for tokio::fs::File {
    async fn opendir(path: impl AsRef<Path>) -> io::Result<File> {
        let path = path.as_ref().to_owned();

        tokio::task::spawn_blocking(move || {
            run_path_with_cstr(&path, &|cstr| unsafe {
                let dir = libc::opendir(cstr.as_ptr());
                if dir.is_null() {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(File::from_raw_fd(dir as c_int))
                }
            })
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "background task failed"))?
    }

    async fn readdir(&self) -> io::Result<ReadDirTokio> {
        let fake_std_inner = FakeStdInnerReadDir {
            dirp: Dir(self.as_raw_fd() as *mut DIR),
            root: PathBuf::new(),
        };
        let fake_std_readdir = FakeStdReadDir {
            #[allow(clippy::arc_with_non_send_sync)]
            inner: Arc::new(fake_std_inner),
            end_of_stream: false,
        };
        let mut std_readdir =
            unsafe { mem::transmute::<FakeStdReadDir, ReadDir>(fake_std_readdir) };

        let mut buf = VecDeque::with_capacity(CHUNK_SIZE);
        let remain = tokio_next_chunk(&mut buf, &mut std_readdir);

        let state = FakeTokioState::Idle(Some((buf, std_readdir, remain)));
        Ok(unsafe { mem::transmute::<FakeTokioReadDir, ReadDirTokio>(FakeTokioReadDir(state)) })
    }
}

#[allow(unused)]
#[derive(Debug)]
enum FakeTokioState {
    Idle(
        Option<(
            VecDeque<io::Result<FakeTokioDirEntry>>,
            std::fs::ReadDir,
            bool,
        )>,
    ),
    Pending(
        JoinHandle<(
            VecDeque<io::Result<FakeTokioDirEntry>>,
            std::fs::ReadDir,
            bool,
        )>,
    ),
}

#[allow(unused)]
struct Dir(*mut libc::DIR);

#[allow(unused)]
pub struct FakeTokioReadDir(FakeTokioState);

#[allow(unused)]
#[derive(Debug)]
pub struct FakeTokioDirEntry {
    #[cfg(not(any(
        target_os = "solaris",
        target_os = "illumos",
        target_os = "haiku",
        target_os = "vxworks",
        target_os = "aix",
        target_os = "nto",
        target_os = "vita",
    )))]
    file_type: Option<FileType>,
    std: Arc<std::fs::DirEntry>,
}

#[allow(unused)]
struct FakeStdInnerReadDir {
    dirp: Dir,
    root: PathBuf,
}

#[allow(unused)]
pub struct FakeStdReadDir {
    inner: Arc<FakeStdInnerReadDir>,
    end_of_stream: bool,
}

fn tokio_next_chunk(
    buf: &mut VecDeque<io::Result<FakeTokioDirEntry>>,
    std: &mut std::fs::ReadDir,
) -> bool {
    for _ in 0..CHUNK_SIZE {
        let ret = match std.next() {
            Some(ret) => ret,
            None => return false,
        };

        let success = ret.is_ok();

        buf.push_back(ret.map(|std| FakeTokioDirEntry {
            #[cfg(not(any(
                target_os = "solaris",
                target_os = "illumos",
                target_os = "haiku",
                target_os = "vxworks",
                target_os = "aix",
                target_os = "nto",
                target_os = "vita",
            )))]
            file_type: std.file_type().ok(),
            std: Arc::new(std),
        }));

        if !success {
            break;
        }
    }

    true
}
