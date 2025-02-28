pub(crate) mod compression;

use std::collections::HashSet;
use std::io::{Error, Read, Write};
use std::path::{Path, PathBuf};
use std::{io, thread};
use tar::{Archive, Builder};

fn chroot(dest: PathBuf) -> Result<(), io::Error> {
    use rustix::thread::*;

    unshare(UnshareFlags::FS)?;
    std::os::unix::fs::chroot(dest)?;
    std::env::set_current_dir("/")?;

    Ok(())
}

pub trait TarSandboxedUnpackExtension {
    async fn unpack_sandboxed<P: AsRef<Path>, E: AsRef<Path>, T: Send + 'static>(
        self,
        chroot_dst: P,
        target_dst: E,
        lock: T,
    ) -> Result<T, io::Error>
    where
        Self: Sized;
}

pub trait TarSandboxedAppendExtension {
    async fn append_dir_all_sandboxed<
        P: AsRef<Path>,
        F: AsRef<Path>,
        E: AsRef<Path>,
        T: Send + 'static,
    >(
        self,
        chroot_dst: P,
        path: F,
        target_dst: E,
        exclude: HashSet<String>,
        lock: T,
        finish: bool,
    ) -> Result<(Self, T), Error>
    where
        Self: Sized;

    fn append_dir_all_sandboxed_blocking<
        P: AsRef<Path>,
        F: AsRef<Path>,
        E: AsRef<Path>,
        T: Send + 'static,
    >(
        self,
        chroot_dst: P,
        path: F,
        target_dst: E,
        exclude: HashSet<String>,
        lock: T,
        finish: bool,
    ) -> Result<(Self, T), Error>
    where
        Self: Sized;
}

impl<R: Read + Send + 'static> TarSandboxedUnpackExtension for Archive<R> {
    async fn unpack_sandboxed<P: AsRef<Path>, E: AsRef<Path>, T: Send + 'static>(
        mut self,
        chroot_dst: P,
        target_dst: E,
        lock: T,
    ) -> Result<T, io::Error> {
        let chroot_dst = chroot_dst.as_ref();
        let chroot_dst = PathBuf::from(chroot_dst);
        let target_dst = target_dst.as_ref();
        let target_dst = PathBuf::from(target_dst);

        tokio::task::spawn_blocking(move || {
            thread::spawn(move || {
                chroot(chroot_dst)?;
                self.unpack(target_dst)?;
                Ok(lock)
            })
            .join()
            .unwrap()
        })
        .await?
    }
}

impl<W: Write + Send + 'static> TarSandboxedAppendExtension for Builder<W> {
    async fn append_dir_all_sandboxed<
        P: AsRef<Path>,
        F: AsRef<Path>,
        E: AsRef<Path>,
        T: Send + 'static,
    >(
        self,
        chroot_dst: P,
        path: F,
        target_dst: E,
        exclude: HashSet<String>,
        lock: T,
        finish: bool,
    ) -> Result<(Self, T), Error>
    where
        Self: Sized,
    {
        let chroot_dst = chroot_dst.as_ref();
        let chroot_dst = PathBuf::from(chroot_dst);
        let path = path.as_ref();
        let path = PathBuf::from(path);
        let target_dst = target_dst.as_ref();
        let target_dst = PathBuf::from(target_dst);

        tokio::task::spawn_blocking(move || {
            self.append_dir_all_sandboxed_blocking(
                chroot_dst, path, target_dst, exclude, lock, finish,
            )
        })
        .await?
    }

    fn append_dir_all_sandboxed_blocking<
        P: AsRef<Path>,
        F: AsRef<Path>,
        E: AsRef<Path>,
        T: Send + 'static,
    >(
        mut self,
        chroot_dst: P,
        path: F,
        target_dst: E,
        _exclude: HashSet<String>,
        lock: T,
        finish: bool,
    ) -> Result<(Self, T), Error>
    where
        Self: Sized,
    {
        let chroot_dst = chroot_dst.as_ref();
        let chroot_dst = PathBuf::from(chroot_dst);
        let path = path.as_ref();
        let path = PathBuf::from(path);
        let target_dst = target_dst.as_ref();
        let target_dst = PathBuf::from(target_dst);

        // TODO: Exclude
        thread::spawn(move || {
            chroot(chroot_dst)?;
            self.append_dir_all(path, target_dst)?;
            if finish {
                self.finish()?;
            }
            Ok((self, lock))
        })
        .join()
        .unwrap()
    }
}
