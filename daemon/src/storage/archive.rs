// SPDX-License-Identifier: AGPL-3.0-only
pub mod compression;
pub mod format;

use std::collections::HashSet;
use std::io::{Error, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::{fs, io, thread};
use tar::{Archive, Builder};
use zip::ZipArchive;

#[cfg(target_os = "linux")]
fn chroot(dest: PathBuf) -> Result<(), io::Error> {
    use rustix::thread::*;

    // SAFETY: Not using UnshareFlags::Files, so should be safe
    unsafe { unshare_unsafe(UnshareFlags::FS)? };
    std::os::unix::fs::chroot(dest)?;
    std::env::set_current_dir("/")?;

    Ok(())
}

#[cfg(target_os = "macos")]
fn chroot_path(chroot_dest: &Path, target_dest: &PathBuf) -> PathBuf {
    target_dest.strip_prefix("/").map_or_else(
        |_err| chroot_dest.join(target_dest),
        |relative| chroot_dest.join(relative),
    )
}

pub trait TarSandboxedUnpackExt {
    async fn unpack_sandboxed<P: AsRef<Path>, E: AsRef<Path>, T: Send + 'static>(
        self,
        chroot_dst: P,
        target_dst: E,
        lock: T,
    ) -> Result<T, io::Error>
    where
        Self: Sized;
}

pub trait ZipSandboxedUnpackExt {
    async fn unpack_sandboxed<P: AsRef<Path>, E: AsRef<Path>, T: Send + 'static>(
        self,
        chroot_dst: P,
        target_dst: E,
        lock: T,
    ) -> Result<T, io::Error>
    where
        Self: Sized;
}

pub trait TarSandboxedAppendExt {
    #[expect(clippy::too_many_arguments)]
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
        follow_symlinks: bool,
    ) -> Result<(Self, T), Error>
    where
        Self: Sized;

    #[expect(clippy::too_many_arguments)]
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
        follow_symlinks: bool,
    ) -> Result<(Self, T), Error>
    where
        Self: Sized;
}

impl<R: Read + Send + 'static> TarSandboxedUnpackExt for Archive<R> {
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
                #[cfg(target_os = "linux")]
                {
                    chroot(chroot_dst)?;
                };
                #[cfg(target_os = "macos")]
                let target_dst = chroot_path(&chroot_dst, &target_dst);
                self.unpack(target_dst)?;
                Ok(lock)
            })
            .join()
            .expect("Archive unpack task panicked")
        })
        .await?
    }
}

impl<R: Read + Seek + Send + 'static> ZipSandboxedUnpackExt for ZipArchive<R> {
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
                #[cfg(target_os = "linux")]
                {
                    chroot(chroot_dst)?;
                };
                #[cfg(target_os = "macos")]
                let target_dst = chroot_path(&chroot_dst, &target_dst);
                self.extract(target_dst).map_err(io::Error::from)?;
                Ok(lock)
            })
            .join()
            .expect("Archive unpack task panicked")
        })
        .await?
    }
}

impl<W: Write + Send + 'static> TarSandboxedAppendExt for Builder<W> {
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
        follow_symlinks: bool,
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
                chroot_dst,
                path,
                target_dst,
                exclude,
                lock,
                finish,
                follow_symlinks,
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
        exclude: HashSet<String>,
        lock: T,
        finish: bool,
        follow_symlinks: bool,
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

        thread::spawn(move || {
            #[cfg(target_os = "linux")]
            {
                chroot(chroot_dst)?;
            };
            #[cfg(target_os = "macos")]
            let target_dst = chroot_path(&chroot_dst, &target_dst);
            append_dir_all(&mut self, &path, &target_dst, &exclude, follow_symlinks)?;
            if finish {
                self.finish()?;
            }
            Ok((self, lock))
        })
        .join()
        .expect("Archive create task panicked")
    }
}

fn append_dir_all<W: Write>(
    archive: &mut Builder<W>,
    path: &Path,
    src_path: &Path,
    exclude: &HashSet<String>,
    follow_symlinks: bool,
) -> io::Result<()> {
    let mut stack = vec![(src_path.to_path_buf(), true, false)];
    while let Some((src, is_dir, is_symlink)) = stack.pop() {
        let dest = path.join(src.strip_prefix(src_path).map_err(io::Error::other)?);
        let mut dest_str = dest.to_string_lossy().to_string();
        if is_dir && !dest_str.ends_with('/') {
            dest_str += "/";
        }
        if exclude.contains(&dest_str) {
            continue;
        }
        // In case of a symlink pointing to a directory, is_dir is false, but src.is_dir() will return true
        if is_dir || (is_symlink && follow_symlinks && src.is_dir()) {
            for entry in fs::read_dir(&src)? {
                let entry = entry?;
                let file_type = entry.file_type()?;
                stack.push((entry.path(), file_type.is_dir(), file_type.is_symlink()));
            }
            if dest != Path::new("") {
                archive.append_dir(&dest, &src)?;
            }
        } else {
            archive.append_path_with_name(&src, &dest)?;
        }
    }
    Ok(())
}
