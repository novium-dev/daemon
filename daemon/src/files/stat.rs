// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::StorageEntry;
use crate::files::{IntoFileEntry as _, storage_path};
use crate::storage::StorageIdentifier;
use crate::storage::archive::format::ArchiveFormat;
use crate::storage::sync::{StorageLockData, StorageTokens, storage_read_timeout};
use crate::{Daemon, DaemonExtra};
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::FileEntry;
use daemon_common::service::files::error_file_not_found::FromFileNotFound;
use daemon_common::service::files::task::{FileOperationKind, StorageTask};
use std::io;
use std::io::ErrorKind;
use std::os::fd::OwnedFd;
use tracing::{debug, error};

/// Cancel safe
#[inline(always)]
pub async fn stat_file<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromFileNotFound,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    path: String,
) -> Result<FileEntry, E> {
    let StorageLockData {
        lock,
        storage,
        tokens,
        ..
    } = storage_read_timeout::<E>(
        &daemon.config,
        &*daemon.conn,
        &daemon.storages,
        id,
        StorageTask::FileOperation {
            kind: FileOperationKind::Stat,
        },
    )
    .await?;
    let result = stat_file_locked::<E>(id, &storage, &tokens, path).await;
    drop(lock);
    result
}

/// Cancel safe, the caller holds the storage lock
pub(super) async fn stat_file_locked<E: From<io::Error> + FromFileNotFound>(
    id: StorageIdentifier,
    storage: &StorageEntry,
    tokens: &StorageTokens,
    path: String,
) -> Result<FileEntry, E> {
    let (storage, _, path) = storage_path(id, storage, path, tokens).await?;
    let metadata = storage
        .symlink_metadata(&path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound | ErrorKind::PermissionDenied => {
                debug!("Failed to stat file (metadata): {err:?}");
            }
            _ => error!("Failed to stat file (metadata): {err:?}"),
        })?;

    let filename = match path.file_name() {
        None => {
            return Err(<E as FromFileNotFound>::from());
        }
        Some(filename) => filename.to_string_lossy().to_string(),
    };
    let mut link_path = None;
    let mut archive = false;
    if metadata.is_symlink() {
        link_path = Some(
            storage
                .read_link(path.clone())
                .await
                .inspect_err(|err| error!("Failed to stat file (read_link): {err:?}"))?
                .to_string_lossy()
                .to_string(),
        );
    } else {
        let file = storage
            .open(&path)
            .await
            .inspect_err(|err| error!("Failed to stat file (open): {err:?}"))?;
        archive = tokio::task::spawn_blocking(move || {
            let fd: OwnedFd = file.into_std().into();
            let file = std::fs::File::from(fd);
            if let Ok(Some(_)) = ArchiveFormat::detect_blocking(file) {
                Ok::<_, io::Error>(true)
            } else {
                Ok(false)
            }
        })
        .await
        .inspect_err(|err| error!("Failed to stat file (detect): {err:?}"))
        .map_err(io::Error::from)?
        .inspect_err(|err| error!("Failed to stat file (detect): {err:?}"))?;
    }

    let relative_path = path.join(filename.clone());
    Ok(metadata.into_fileentry(
        filename,
        relative_path.to_string_lossy().to_string(),
        link_path,
        archive,
    ))
}
