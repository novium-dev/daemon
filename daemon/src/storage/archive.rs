// SPDX-License-Identifier: AGPL-3.0-only
pub mod compression;
pub mod format;

use async_trait::async_trait;
use axum_extra::headers::ContentLength;
use axum_extra::response::Attachment;
use chrono::{TimeZone as _, Utc};
use daemon_common::service::DataStream;
use daemon_common::service::backup::BackupResponseEntry;
use daemon_common::service::files::task::{AtomicProgress, StorageTask};
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::collections::HashSet;
use std::io::{Error, ErrorKind, Read, Seek, Write};
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::{fs, io, thread};
use tar::{Archive, Builder};
use tokio::fs::File;
use tracing::{debug, error};
use zip::ZipArchive;

use crate::config::{ArchiveCompression, StorageEntry};
use crate::storage::archive::format::ArchiveFormat;
use crate::storage::backup::download::backup_filename;
use crate::storage::backup::{BackupBackend, BackupMetadataExt as _};
use crate::storage::{self, StorageIdentifier};
use crate::sync::task::{TaskOwnedRwLockReadGuard, TaskOwnedRwLockWriteGuard};
use crate::utils::read_dir_opt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveBackupBackend {
    pub compression: ArchiveCompression,
}

#[async_trait]
impl BackupBackend for ArchiveBackupBackend {
    fn target_path(&self, id: StorageIdentifier, base: &Path, backup_id: i64) -> PathBuf {
        let target_dir = base.join(id.as_pathbuf());
        target_dir.join(format!("{backup_id}.tar{}", self.compression.ext()))
    }

    fn compatible(&self, _storage: &StorageEntry) -> bool {
        true
    }

    async fn list(
        &self,
        storage: &str,
        id: StorageIdentifier,
        base: &Path,
        entries: &mut Vec<BackupResponseEntry>,
    ) -> Result<(), io::Error> {
        let target_dir = base.join(id.as_pathbuf());
        let Some(mut readdir) = read_dir_opt(&target_dir)
            .await
            .inspect_err(|err| error!("Failed to list backups (read_dir): {err:?}"))?
        else {
            return Ok(());
        };
        while let Some(entry) = readdir
            .next_entry()
            .await
            .inspect_err(|err| error!("Failed to list backups (next_entry): {err:?}"))?
        {
            let metadata = entry.metadata().await.inspect_err(|err| {
                error!("Failed to list backups (metadata): {err:?}");
            })?;

            #[expect(
                clippy::filetype_is_file,
                reason = "Only list regular files here, not device files etc."
            )]
            if !metadata.file_type().is_file() {
                continue;
            }
            let entry_path = entry.path();
            let size = metadata.size();
            let filename = entry.file_name().to_string_lossy().to_string();
            let Some(entry) = filename.strip_suffix(&format!(".tar{}", self.compression.ext()))
            else {
                continue;
            };
            let entry: Result<i64, _> = entry.parse();

            let file = File::open(entry_path)
                .await
                .inspect_err(|err| error!("Failed to list backups (open): {err:?}"))?;
            let name = file.get_backup_name().await.inspect_err(|err| {
                error!("Failed to list backups (get_name): {err:?}");
            })?;
            let protected = file.get_backup_protected().await.inspect_err(|err| {
                error!("Failed to list backups (get_protected): {err:?}");
            })?;
            if let Ok(entry) = entry {
                entries.push(BackupResponseEntry {
                    time: entry,
                    storage: storage.to_owned(),
                    size: Some(size),
                    name,
                    protected,
                });
            }
        }
        Ok(())
    }

    async fn create(
        &self,
        id: StorageIdentifier,
        base: &Path,
        backup_id: i64,
        _storage: &StorageEntry,
        storage_target: &Path,
        mut lock: TaskOwnedRwLockReadGuard<StorageTask, ()>,
        exclude: HashSet<String>,
    ) -> Result<TaskOwnedRwLockReadGuard<StorageTask, ()>, io::Error> {
        let target_dir = base.join(id.as_pathbuf());

        if !tokio::fs::try_exists(&target_dir)
            .await
            .inspect_err(|err| error!("Backup creation failed (try_exists): {:?}", err))?
        {
            tokio::fs::create_dir_all(&target_dir)
                .await
                .inspect_err(|err| error!("Backup creation failed (create_dir_all): {:?}", err))?;
        }

        lock = tokio::task::spawn(create_archive_backup(
            storage_target.to_path_buf(),
            target_dir,
            backup_id,
            lock,
            exclude,
            self.compression,
        ))
        .await
        .map_err(io::Error::from)??;
        Ok(lock)
    }

    async fn delete(
        &self,
        id: StorageIdentifier,
        base: &Path,
        backup_id: i64,
    ) -> Result<(), io::Error> {
        tokio::fs::remove_file(self.target_path(id, base, backup_id))
            .await
            .inspect_err(|err| {
                if err.kind() == ErrorKind::NotFound {
                    debug!("Failed to delete a backup (remove_file): {err:?}");
                } else {
                    error!("Failed to delete a backup (remove_file): {err:?}");
                }
            })
    }

    async fn restore(
        &self,
        id: StorageIdentifier,
        base: &Path,
        backup_id: i64,
        storage: &StorageEntry,
        storage_target: &Path,
        lock: &mut TaskOwnedRwLockWriteGuard<StorageTask, ()>,
        disk: Option<u64>,
        value: Arc<(AtomicI64, AtomicI64)>,
    ) -> Result<(), io::Error> {
        let target = self.target_path(id, base, backup_id);
        let file = File::open(target).await.inspect_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                debug!("Failed to restore backup (open): {err:?}");
            } else {
                error!("Failed to restore backup (open): {err:?}");
            }
        })?;
        let meta = file
            .metadata()
            .await
            .inspect_err(|err| error!("Unable to restore backup (metadata): {err:?}"))?;
        let file = file.into_std().await;
        let progress_arc = lock.task_mut(|task| {
            let StorageTask::BackupRestore { progress, .. } = task else {
                unreachable!();
            };
            let progress_arc = Arc::new(AtomicProgress {
                current: AtomicU64::new(0),
                max: AtomicU64::new(meta.len()),
            });
            *progress = Some(Arc::clone(&progress_arc));
            progress_arc
        });
        let wrapper = BackupRestoreTarReadProgressWrapper {
            inner: file,
            func: move |read| {
                progress_arc
                    .current
                    .fetch_add(read as u64, Ordering::Relaxed);
            },
        };

        let decoder = self
            .compression
            .create_decoder(wrapper)
            .inspect_err(|err| {
                error!("Failed to restore backup (create_decoder): {err:?}");
            })?;
        let archive = Archive::new(decoder);
        storage::delete(storage, id).await.inspect_err(|err| {
            error!("Failed to restore backup (storage_delete): {err:?}");
        })?;
        storage::prepare(id, storage, &value, disk)
            .await
            .inspect_err(|err| {
                error!("Failed to restore backup (storage_prepare): {err:?}");
            })?;
        archive
            .unpack_sandboxed(storage_target, "/", ())
            .await
            .inspect_err(|err| error!("Failed to restore backup (unpack): {err:?}"))?;
        Ok(())
    }

    async fn download(
        &self,
        id: StorageIdentifier,
        base: &Path,
        backup_id: i64,
        _exclude: HashSet<String>,
    ) -> Result<Attachment<DataStream>, io::Error> {
        let target = self.target_path(id, base, backup_id);

        let file = File::open(target).await.inspect_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                debug!("Failed to create a backup stream (open): {err:?}");
            } else {
                error!("Failed to create a backup stream (open): {err:?}");
            }
        })?;
        let length = file
            .metadata()
            .await
            .inspect_err(|err| error!("Failed to create a backup stream (metadata): {err:?}"))?
            .len();

        Ok(
            Attachment::new(DataStream::by_read(file, Some(ContentLength(length))))
                .content_type(ArchiveFormat::Tar(self.compression).content_type())
                .filename(format!(
                    "{}_{}.tar{}",
                    backup_filename(id),
                    Utc.timestamp_millis_opt(backup_id).unwrap().to_rfc3339(),
                    self.compression.ext()
                )),
        )
    }
}

#[inline(always)]
async fn create_archive_backup(
    storage_target: PathBuf,
    target_dir: PathBuf,
    timestamp: i64,
    lock: TaskOwnedRwLockReadGuard<StorageTask, ()>,
    exclude: HashSet<String>,
    compression: ArchiveCompression,
) -> Result<TaskOwnedRwLockReadGuard<StorageTask, ()>, io::Error> {
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(target_dir.join(format!("{}.tar{}", timestamp, compression.ext())))
        .await
        .inspect_err(|err| debug!("Backup creation failed (open): {err:?}"))?;
    let file = file.into_std().await;

    let progress_arc = lock.task_mut(|task| {
        let StorageTask::BackupCreate { progress, .. } = task else {
            unreachable!();
        };
        let progress_arc = Arc::new(AtomicU64::new(0));
        *progress = Some(Arc::clone(&progress_arc));
        progress_arc
    });
    let wrapper = BackupCreateTarWriteProgressWrapper {
        inner: file,
        func: move |read| {
            progress_arc.fetch_add(read as u64, Ordering::Relaxed);
        },
    };

    let encoder = compression
        .create_encoder(wrapper)
        .inspect_err(|err| error!("Backup creation failed (create_encoder): {err:?}"))?;
    let mut archive = tar::Builder::new(encoder);
    archive.follow_symlinks(false);
    let (archive, ()) = archive
        .append_dir_all_sandboxed(&storage_target, "", "/", exclude, (), true, false)
        .await
        .inspect_err(|err| error!("Backup creation failed (append_dir_all): {err:?}"))?;
    tokio::task::spawn_blocking(move || {
        archive
            .into_inner()
            .expect("Archive should have already been finished? Bug?")
            .finish()?
            .flush()
    })
    .await
    .inspect_err(|err| error!("Backup creation failed (spawn_blocking): {err:?}"))
    .map_err(io::Error::from)?
    .inspect_err(|err| error!("Backup creation failed (flush): {err:?}"))?;
    Ok(lock)
}

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

pub(crate) trait ArchiveSandboxedUnpackExt {
    async fn unpack_sandboxed<P: AsRef<Path>, E: AsRef<Path>, T: Send + 'static>(
        self,
        chroot_dst: P,
        target_dst: E,
        lock: T,
    ) -> Result<T, io::Error>
    where
        Self: Sized;
}

pub(crate) trait TarSandboxedAppendExt {
    #[expect(clippy::too_many_arguments)]
    async fn append_dir_all_sandboxed<
        P: AsRef<Path>,
        F: AsRef<Path>,
        E: AsRef<Path>,
        T: Send + 'static,
        X: Borrow<HashSet<String>> + Send + 'static,
    >(
        self,
        chroot_dst: P,
        path: F,
        target_dst: E,
        exclude: X,
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
        X: Borrow<HashSet<String>> + Send + 'static,
    >(
        self,
        chroot_dst: P,
        path: F,
        target_dst: E,
        exclude: X,
        lock: T,
        finish: bool,
        follow_symlinks: bool,
    ) -> Result<(Self, T), Error>
    where
        Self: Sized;

    async fn append_path_sandboxed<
        P: AsRef<Path>,
        F: AsRef<Path>,
        E: AsRef<Path>,
        T: Send + 'static,
    >(
        self,
        chroot_dst: P,
        path: F,
        target_dst: E,
        lock: T,
    ) -> Result<(Self, T), Error>
    where
        Self: Sized;
}

impl<R: Read + Send + 'static> ArchiveSandboxedUnpackExt for Archive<R> {
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

impl<R: Read + Seek + Send + 'static> ArchiveSandboxedUnpackExt for ZipArchive<R> {
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
        X: Borrow<HashSet<String>> + Send + 'static,
    >(
        self,
        chroot_dst: P,
        path: F,
        target_dst: E,
        exclude: X,
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
        X: Borrow<HashSet<String>> + Send + 'static,
    >(
        mut self,
        chroot_dst: P,
        path: F,
        target_dst: E,
        exclude: X,
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
            append_dir_all(
                &mut self,
                &path,
                &target_dst,
                exclude.borrow(),
                follow_symlinks,
            )?;
            if finish {
                self.finish()?;
            }
            Ok((self, lock))
        })
        .join()
        .expect("Archive create task panicked")
    }

    async fn append_path_sandboxed<
        P: AsRef<Path>,
        F: AsRef<Path>,
        E: AsRef<Path>,
        T: Send + 'static,
    >(
        mut self,
        chroot_dst: P,
        path: F,
        target_dst: E,
        lock: T,
    ) -> Result<(Self, T), Error>
    where
        Self: Sized,
    {
        let chroot_dst = PathBuf::from(chroot_dst.as_ref());
        let path = PathBuf::from(path.as_ref());
        let target_dst = PathBuf::from(target_dst.as_ref());

        tokio::task::spawn_blocking(move || {
            thread::spawn(move || {
                #[cfg(target_os = "linux")]
                {
                    chroot(chroot_dst)?;
                };
                #[cfg(target_os = "macos")]
                let target_dst = chroot_path(&chroot_dst, &target_dst);
                self.append_path_with_name(&target_dst, &path)?;
                Ok((self, lock))
            })
            .join()
            .expect("Archive create task panicked")
        })
        .await?
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

struct BackupRestoreTarReadProgressWrapper<R: Read, F: FnMut(usize)> {
    inner: R,
    func: F,
}

impl<R: Read, F: FnMut(usize)> Read for BackupRestoreTarReadProgressWrapper<R, F> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        (self.func)(read);
        Ok(read)
    }
}

struct BackupCreateTarWriteProgressWrapper<W: Write, F: FnMut(usize)> {
    inner: W,
    func: F,
}

impl<W: Write, F: FnMut(usize)> Write for BackupCreateTarWriteProgressWrapper<W, F> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buf)?;
        (self.func)(written);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}
