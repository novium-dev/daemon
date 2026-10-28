// SPDX-License-Identifier: AGPL-3.0-only
//! Low impact storage size calculation.
//!
//! The scan runs on its own thread with the lowest CPU and IO priority, works
//! in short slices and only holds the storage lock during a slice. Tasks that
//! need exclusive access to a storage (restore, wipe) therefore never wait
//! longer than one slice, and abort the scan instead of failing themselves.
use crate::sync::task::TaskRwLock;
use core::time::Duration;
use daemon_common::service::files::task::StorageTask;
use std::collections::HashSet;
use std::fs::{self, ReadDir};
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use tracing::warn;

/// How long a scan may work before it releases the storage lock and pauses.
const SCAN_SLICE: Duration = Duration::from_millis(20);
/// How long a scan pauses after each slice, bounding its share of CPU and IO.
const SCAN_PAUSE: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub enum ScanError {
    /// Another task required the storage, the scan was aborted.
    Interrupted,
    IO(io::Error),
}

impl From<io::Error> for ScanError {
    fn from(err: io::Error) -> Self {
        Self::IO(err)
    }
}

pub type ScanTarget<K> = (K, PathBuf, Arc<TaskRwLock<StorageTask, ()>>);

/// Scans all given storages on a dedicated low priority thread.
pub fn scan_all<K: Copy + Send + Sync>(
    storages: &[ScanTarget<K>],
) -> Vec<(K, Result<u64, ScanError>)> {
    let scan = || {
        storages
            .iter()
            .map(|(key, path, lock)| (*key, dir_size(path, lock)))
            .collect()
    };
    thread::scope(|scope| {
        match thread::Builder::new()
            .name("storage-scan".to_owned())
            .spawn_scoped(scope, || {
                lower_priority();
                scan()
            }) {
            Ok(handle) => handle.join().unwrap_or_default(),
            Err(err) => {
                warn!("Unable to spawn storage scan thread, scanning in place: {err:?}");
                scan()
            }
        }
    })
}

/// Lowers the priority of the calling thread to idle for IO and lowest for CPU.
#[cfg(target_os = "linux")]
fn lower_priority() {
    const IOPRIO_WHO_PROCESS: libc::c_long = 1;
    const IOPRIO_CLASS_IDLE: libc::c_long = 3;
    const IOPRIO_CLASS_SHIFT: libc::c_long = 13;
    const CALLING_THREAD: libc::c_long = 0;

    // SAFETY: Only changes the scheduling priority of the calling thread (who = 0).
    if unsafe {
        libc::syscall(
            libc::SYS_ioprio_set,
            IOPRIO_WHO_PROCESS,
            CALLING_THREAD,
            IOPRIO_CLASS_IDLE << IOPRIO_CLASS_SHIFT,
        )
    } < 0
    {
        warn!(
            "Unable to lower io priority of storage scan: {:?}",
            io::Error::last_os_error()
        );
    }
    // SAFETY: On linux the nice value is per thread, so this only affects the calling thread.
    if unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, 19) } < 0i32 {
        warn!(
            "Unable to lower cpu priority of storage scan: {:?}",
            io::Error::last_os_error()
        );
    }
}

#[cfg(not(target_os = "linux"))]
const fn lower_priority() {}

/// Calculates the apparent size of all files below `path`.
///
/// Symlinks are not followed and hard links are only counted once. Entries
/// removed while scanning are skipped.
pub fn dir_size(path: &Path, lock: &TaskRwLock<StorageTask, ()>) -> Result<u64, ScanError> {
    let mut size = 0u64;
    let mut seen = HashSet::new();
    let mut pending: Vec<PathBuf> = Vec::new();
    let mut current: Option<ReadDir> = Some(fs::read_dir(path)?);

    loop {
        let Ok(guard) = lock.try_read(StorageTask::Scan) else {
            return Err(ScanError::Interrupted);
        };
        let start = Instant::now();
        while start.elapsed() < SCAN_SLICE {
            let entries = match current.as_mut() {
                Some(entries) => entries,
                None => {
                    let Some(dir) = pending.pop() else {
                        return Ok(size);
                    };
                    match skip_removed(fs::read_dir(dir))? {
                        Some(entries) => current.insert(entries),
                        None => continue,
                    }
                }
            };
            let Some(entry) = entries.next() else {
                current = None;
                continue;
            };
            let Some(entry) = skip_removed(entry)? else {
                continue;
            };
            // Does not follow symlinks
            let Some(metadata) = skip_removed(entry.metadata())? else {
                continue;
            };
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.nlink() <= 1 || seen.insert((metadata.dev(), metadata.ino())) {
                size = size.saturating_add(metadata.len());
            } else {
                // Hard link already counted
            }
        }
        drop(guard);
        thread::sleep(SCAN_PAUSE);
    }
}

fn skip_removed<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::{ScanError, dir_size};
    use crate::sync::task::TaskRwLock;
    use daemon_common::service::files::task::StorageTask;
    use std::fs;
    use std::os::unix::fs::symlink;

    #[test]
    fn counts_files_once_and_skips_symlinks() {
        let root = std::env::temp_dir().join(format!("daemon-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a/b")).expect("create dirs");
        fs::write(root.join("file"), [0u8; 100]).expect("write file");
        fs::write(root.join("a/b/file"), [0u8; 50]).expect("write nested file");
        fs::hard_link(root.join("file"), root.join("a/hardlink")).expect("hard link");
        symlink(root.join("a"), root.join("a/b/loop")).expect("symlink");

        let lock = TaskRwLock::<StorageTask, ()>::default();
        let size = dir_size(&root, &lock).expect("scan");
        let link_len = fs::symlink_metadata(root.join("a/b/loop"))
            .expect("symlink metadata")
            .len();
        assert_eq!(size, 150 + link_len);

        let guard = lock.try_write(StorageTask::Wipe).expect("write lock");
        assert!(matches!(
            dir_size(&root, &lock),
            Err(ScanError::Interrupted)
        ));
        drop(guard);

        fs::remove_dir_all(&root).expect("cleanup");
    }
}
