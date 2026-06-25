// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::{BackupEntry, StorageEntry};
use crate::storage::StorageIdentifier;
use crate::storage::archive::TarSandboxedUnpackExt as _;
use crate::storage::sync::{StorageLockData, storage_write_timeout};
use crate::sync::task::TaskOwnedRwLockWriteGuard;
use crate::{Daemon, DaemonExtra, storage};
use core::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use daemon_common::service::backup::error_backup_storage_location_not_found::FromBackupStorageLocationNotFound;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::task::{AtomicProgress, BackupLocation, StorageTask};
use std::io;
use std::io::ErrorKind;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use tar::Archive;
use tokio::fs::File;
use tracing::{debug, error};

/// Cancel safe
pub async fn restore_backup<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromBackupStorageLocationNotFound
        + Send
        + 'static,
>(
    state: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    backup_id: i64,
    backup_storage: &String,
) -> Result<(), E> {
    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or_else(<E as FromBackupStorageLocationNotFound>::from)
        .inspect_err(|_err| debug!("Backup storage does not exist: {backup_storage}"))?
        .clone();

    let StorageLockData {
        path,
        lock,
        disk,
        storage,
        value: storage_value,
        ..
    } = storage_write_timeout::<E>(
        &state.config,
        &*state.conn,
        &state.storages,
        id,
        StorageTask::BackupRestore {
            location: BackupLocation {
                storage: backup_storage.clone(),
                id: backup_id,
            },
            progress: None,
        },
    )
    .await?;

    let lock = tokio::task::spawn(restore_backup0::<E>(
        id,
        storage,
        path,
        backup_id,
        backup,
        lock,
        disk,
        Arc::clone(&storage_value),
    ))
    .await
    .map_err(io::Error::from)??;
    storage_value.0.store(-1, Ordering::Relaxed);
    drop(lock);
    Ok(())
}

#[inline(always)]
#[expect(clippy::too_many_arguments)]
async fn restore_backup0<E: From<io::Error>>(
    id: StorageIdentifier,
    storage: StorageEntry,
    path: String,
    backup_id: i64,
    backup: BackupEntry,
    lock: TaskOwnedRwLockWriteGuard<StorageTask, ()>,
    disk: Option<u64>,
    value: Arc<(AtomicI64, AtomicI64)>,
) -> Result<TaskOwnedRwLockWriteGuard<StorageTask, ()>, E> {
    let dir = PathBuf::from(path);
    let target_dir = PathBuf::from(backup.path()).join(id.as_pathbuf());
    match backup {
        BackupEntry::Archive { compression, .. } => {
            let target = target_dir.join(format!("{backup_id}.tar{}", compression.ext()));
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

            let decoder = compression.create_decoder(wrapper).inspect_err(|err| {
                error!("Failed to restore backup (create_decoder): {err:?}");
            })?;
            let archive = Archive::new(decoder);
            storage::delete(&storage, id).await.inspect_err(|err| {
                error!("Failed to restore backup (storage_delete): {err:?}");
            })?;
            storage::prepare(id, &storage, &value, disk)
                .await
                .inspect_err(|err| {
                    error!("Failed to restore backup (storage_prepare): {err:?}");
                })?;
            archive
                .unpack_sandboxed(&dir, "/", ())
                .await
                .inspect_err(|err| error!("Failed to restore backup (unpack): {err:?}"))?;
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs { .. } => {
            use std::io::ErrorKind;
            let target = target_dir.join(backup_id.to_string());
            let parent = dir.join("..");
            let fd = tokio::fs::File::open(&target).await.inspect_err(|err| {
                if err.kind() == ErrorKind::NotFound {
                    debug!("Failed to restore backup (open): {err:?}");
                } else {
                    error!("Failed to restore backup (open): {err:?}");
                }
            })?;
            let Some(filename) = dir.file_name() else {
                error!("Missing final component in storage path");
                return Err(E::from(io::Error::new(
                    ErrorKind::InvalidData,
                    "No final component in storage path",
                )));
            };
            let Some(filename) = filename.to_str() else {
                error!("Final component in storage path is not utf8");
                return Err(E::from(io::Error::new(
                    ErrorKind::InvalidData,
                    "Final component in storage path is not utf8",
                )));
            };
            super::super::btrfs::remove_subvolume(&parent, filename)
                .await
                .inspect_err(|err| error!("Failed to restore backup (delete_snapshot): {err:?}"))?;
            super::super::btrfs::new_snapshot(fd, &parent, filename, false)
                .await
                .inspect_err(|err| error!("Failed to restore backup (create_snapshot): {err:?}"))?;
        }
    }
    Ok(lock)
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
