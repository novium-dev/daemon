// Own implementation of directory handles with libc and tokio, because rust doesn't support these yet
// https://github.com/rust-lang/rust/issues/120426

mod small_c_string;

use crate::dirfd::small_c_string::run_path_with_cstr;
use libc::c_int;
use log::debug;
use std::io;
use std::os::fd::FromRawFd;
use std::path::Path;
use tokio::fs::File;

pub(crate) trait DirectoryFDExt {
    async fn opendir(path: impl AsRef<Path>) -> io::Result<File>;
}

impl DirectoryFDExt for tokio::fs::File {
    async fn opendir(path: impl AsRef<Path>) -> io::Result<File> {
        debug!("Unsafe opendir");
        let path = path.as_ref().to_owned();

        tokio::task::spawn_blocking(move || {
            run_path_with_cstr(&path, &|cstr| unsafe {
                let dir = libc::opendir(cstr.as_ptr());
                if (dir as c_int) < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(File::from_raw_fd(dir as c_int))
                }
            })
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "background task failed"))?
    }
}
