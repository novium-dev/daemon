/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
pub mod compression;

use std::collections::HashSet;
use std::io::{Error, Read, Write};
use std::path::{Path, PathBuf};
use std::{fs, io, thread};
use tar::{Archive, Builder, HeaderMode};

fn chroot(dest: PathBuf) -> Result<(), io::Error> {
    use rustix::thread::*;

    // SAFETY: Should be safe
    unsafe { unshare_unsafe(UnshareFlags::FS)? };
    std::os::unix::fs::chroot(dest)?;
    std::env::set_current_dir("/")?;

    Ok(())
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

pub trait TarSandboxedAppendExt {
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
                chroot(chroot_dst)?;
                self.unpack(target_dst)?;
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

        thread::spawn(move || {
            chroot(chroot_dst)?;
            append_dir_all(&mut self, &path, &target_dst, &exclude)?;
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
) -> io::Result<()> {
    #[expect(dead_code, reason = "this is an exact copy of Builder from upstream")]
    struct BuilderCopy<W: Write> {
        options: BuilderOptionsCopy,
        finished: bool,
        obj: Option<W>,
    }
    #[derive(Clone, Copy)]
    #[expect(
        dead_code,
        reason = "this is an exact copy of BuilderOptions from upstream"
    )]
    struct BuilderOptionsCopy {
        mode: HeaderMode,
        follow: bool,
        sparse: bool,
    }

    let archive_raw_copy = core::ptr::from_ref(archive).cast::<BuilderCopy<W>>();

    // SAFETY: As long as the struct matches if the upstream version, there is no undefined behaviour
    let follow_symlinks = unsafe { &*archive_raw_copy }.options.follow;

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
