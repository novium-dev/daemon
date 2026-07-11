// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::{ArchiveCompression, BackupEntry, StorageEntry};
use crate::storage::StorageIdentifier;
use crate::storage::archive::TarSandboxedAppendExt as _;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::sync::task::TaskOwnedRwLockReadGuard;
use crate::{Daemon, DaemonExtra};
use chrono::Utc;
use core::sync::atomic::{AtomicU64, Ordering};
use daemon_common::service::backup::error_backup_storage_incompatible::FromBackupStorageIncompatible;
use daemon_common::service::backup::error_backup_storage_location_not_found::FromBackupStorageLocationNotFound;
use daemon_common::service::backup::error_backup_storage_no_suitable_found::FromBackupStorageNoSuitableFound;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::backup::BackupResponseEntry;
use daemon_common::service::files::task::{BackupLocation, StorageTask};
use std::collections::HashSet;
use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::debug;

/// Cancel safe
pub async fn create_backup_order<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromBackupStorageNoSuitableFound,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    name: Option<String>,
) -> Result<BackupResponseEntry, E> {
    let StorageLockData {
        path,
        lock,
        backup_exclude,
        storage,
        ..
    } = storage_read_timeout::<E>(
        &daemon.config,
        &*daemon.conn,
        &daemon.storages,
        id,
        StorageTask::BackupCreate {
            location: None,
            progress: None,
        },
    )
    .await?;

    let mut storages: Vec<(String, i8, &BackupEntry)> = daemon
        .config
        .backup
        .iter()
        .map(|(name, entry)| (name.clone(), entry.order(), entry))
        .filter_map(|(name, order, entry)| order.map(|order| (name, order, entry)))
        .collect();
    storages.sort_by_key(|storage| storage.1);
    let mut suitable = None;
    for (storage_name, _order, entry) in storages {
        if compatible(&storage, entry) {
            suitable = Some((storage_name, entry));
            break;
        }
    }
    match suitable {
        None => Err(<E as FromBackupStorageNoSuitableFound>::from()),
        Some((backup_storage, backup_entry)) => {
            let (entry, lock) = create_backup0::<E>(
                path,
                id,
                backup_storage,
                backup_entry,
                lock,
                backup_exclude,
                name,
            )
            .await?;
            drop(lock); // Ensure lock hasn't been unlocked until now
            Ok(entry)
        }
    }
}

#[cfg_attr(
    not(feature = "btrfs"),
    expect(
        clippy::missing_const_for_fn,
        reason = "non-const fn .eq used with btrfs"
    )
)]
fn compatible(storage: &StorageEntry, backup: &BackupEntry) -> bool {
    match (storage, backup) {
        #[cfg(feature = "btrfs")]
        (StorageEntry::Btrfs { group: group1, .. }, BackupEntry::Btrfs { group: group2, .. })
            if group1.eq(group2) =>
        {
            true
        }
        (_, BackupEntry::Archive { .. } | BackupEntry::Rustic { .. }) => true,
        #[cfg(feature = "btrfs")]
        _ => false,
    }
}

/// Cancel safe
pub async fn create_backup<
    E: From<std::io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromBackupStorageIncompatible
        + FromBackupStorageLocationNotFound,
>(
    state: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    backup_storage: &String,
    name: Option<String>,
) -> Result<BackupResponseEntry, E> {
    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or_else(<E as FromBackupStorageLocationNotFound>::from)
        .inspect_err(|_err| debug!("Backup storage does not exist: {backup_storage}"))?;
    let StorageLockData {
        path,
        lock,
        backup_exclude,
        storage,
        ..
    } = storage_read_timeout::<E>(
        &state.config,
        &*state.conn,
        &state.storages,
        id,
        StorageTask::BackupCreate {
            location: None,
            progress: None,
        },
    )
    .await?;

    if !compatible(&storage, backup) {
        return Err(<E as FromBackupStorageIncompatible>::from());
    }

    let (entry, lock) = create_backup0::<E>(
        path,
        id,
        backup_storage.clone(),
        backup,
        lock,
        backup_exclude,
        name,
    )
    .await?;
    drop(lock);
    Ok(entry)
}

/// Cancel safe
async fn create_backup0<E: From<std::io::Error>>(
    path: String,
    id: StorageIdentifier,
    storage: String,
    backup: &BackupEntry,
    mut lock: TaskOwnedRwLockReadGuard<StorageTask, ()>,
    mut exclude: HashSet<String>,
    name: Option<String>,
) -> Result<(BackupResponseEntry, TaskOwnedRwLockReadGuard<StorageTask, ()>), E> {
    exclude.insert(".tmp/".to_owned());

    let dir = PathBuf::from(path);
    let target_dir = PathBuf::from(backup.path()).join(id.as_pathbuf());
    if !tokio::fs::try_exists(&target_dir)
        .await
        .inspect_err(|err| debug!("Backup creation failed (try_exists): {:?}", err))?
    {
        tokio::fs::create_dir_all(&target_dir)
            .await
            .inspect_err(|err| debug!("Backup creation failed (create_dir_all): {:?}", err))?;
    }
    let timestamp = Utc::now().timestamp_millis();

    lock.task_mut(|task| {
        let StorageTask::BackupCreate { location, .. } = task else {
            unreachable!();
        };
        *location = Some(BackupLocation {
            storage: storage.clone(),
            id: timestamp,
        });
    });

    let size;
    // Backup type where the name lives in an extended attribute of a backing file/subvolume.
    let named_backing;
    match backup {
        BackupEntry::Archive { compression, .. } => {
            let result;
            (lock, result) = tokio::task::spawn(create_archive_backup(
                dir,
                target_dir.clone(),
                timestamp,
                lock,
                exclude,
                *compression,
            ))
            .await
            .map_err(io::Error::from)??;
            size = Some(result);
            named_backing =
                Some(target_dir.join(format!("{}.tar{}", timestamp, compression.ext())));
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { .. } => {
            let subvol_fd = tokio::fs::File::open(&dir)
                .await
                .inspect_err(|err| debug!("Backup creation failed (open): {:?}", err))?;

            super::super::btrfs::new_snapshot(subvol_fd, &target_dir, &timestamp.to_string(), true)
                .await
                .inspect_err(|err| debug!("Backup creation failed (snapshot): {:?}", err))?;
            size = None;
            named_backing = Some(target_dir.join(timestamp.to_string()));
        }
        BackupEntry::Rustic { password, .. } => {
            let password = password.clone();
            let name = name.clone();
            size = tokio::task::spawn_blocking(move || {
                super::rustic::create(&target_dir, password, &dir, timestamp, name)
            })
            .await
            .map_err(io::Error::from)?
            .inspect_err(|err| debug!("Backup creation failed (rustic): {err:?}"))?;
            named_backing = None;
        }
    }
    if name.is_some()
        && let Some(backing) = named_backing
    {
        apply_backup_name(backing, name.clone())
            .await
            .inspect_err(|err| debug!("Backup creation failed (set name): {err:?}"))?;
    }
    Ok((
        BackupResponseEntry {
            time: timestamp,
            storage,
            name,
            size,
        },
        lock,
    ))
}

/// Stores the backup name in the extended attribute of the backup's backing file.
///
/// Cancel safe
async fn apply_backup_name(target: PathBuf, name: Option<String>) -> Result<(), io::Error> {
    use super::BackupNameExt as _;

    let file = tokio::fs::File::open(target).await?;
    file.set_backup_name(name).await
}

#[inline(always)]
async fn create_archive_backup(
    dir: PathBuf,
    target_dir: PathBuf,
    timestamp: i64,
    lock: TaskOwnedRwLockReadGuard<StorageTask, ()>,
    exclude: HashSet<String>,
    compression: ArchiveCompression,
) -> Result<(TaskOwnedRwLockReadGuard<StorageTask, ()>, u64), io::Error> {
    let target = target_dir.join(format!("{}.tar{}", timestamp, compression.ext()));
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&target)
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
        .inspect_err(|err| debug!("Backup creation failed (create_encoder): {err:?}"))?;
    let mut archive = tar::Builder::new(encoder);
    archive.follow_symlinks(false);
    let (archive, ()) = archive
        .append_dir_all_sandboxed(&dir, "", "/", exclude, (), true, false)
        .await
        .inspect_err(|err| debug!("Backup creation failed (append_dir_all): {err:?}"))?;
    tokio::task::spawn_blocking(move || {
        archive
            .into_inner()
            .expect("Archive should have already been finished? Bug?")
            .finish()?
            .flush()
    })
    .await
    .inspect_err(|err| debug!("Backup creation failed (spawn_blocking): {err:?}"))
    .map_err(io::Error::from)?
    .inspect_err(|err| debug!("Backup creation failed (flush): {err:?}"))?;
    let size = tokio::fs::metadata(&target)
        .await
        .inspect_err(|err| debug!("Backup creation failed (metadata): {err:?}"))?
        .len();
    Ok((lock, size))
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
