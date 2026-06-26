// SPDX-License-Identifier: AGPL-3.0-only
use crate::files::{IntoFileEntry as _, storage_path};
use crate::storage::StorageIdentifier;
use crate::storage::archive::compression::ArchiveCompression;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::{Daemon, DaemonExtra};
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::FileEntry;
use daemon_common::service::files::task::{FileOperationKind, StorageTask};
use std::io;
use std::io::ErrorKind;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use tracing::{debug, error};

/// Cancel safe
#[inline(always)]
pub async fn list_files<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure,
>(
    state: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    path: String,
) -> Result<Vec<FileEntry>, E> {
    let StorageLockData {
        lock,
        storage,
        tokens,
        ..
    } = storage_read_timeout::<E>(
        &state.config,
        &*state.conn,
        &state.storages,
        id,
        StorageTask::FileOperation {
            kind: FileOperationKind::List,
        },
    )
    .await?;
    let (storage, _, path) = storage_path(id, &storage, path, &tokens).await?;
    let read_dir = storage
        .read_dir(&path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound | ErrorKind::PermissionDenied | ErrorKind::NotADirectory => {
                debug!("Failed to list files (read_dir): {err:?}");
            }
            _ => error!("Failed to list files (read_dir): {err:?}"),
        })?;
    let mut files = Vec::new();
    for entry in read_dir {
        let entry = entry.inspect_err(|err| error!("Failed to list files (entry): {err:?}"))?;
        let metadata = entry
            .metadata()
            .inspect_err(|err| error!("Failed to list files (metadata): {err:?}"))?;
        let mut link_path = None;
        let mut archive = false;

        let relative_path = path.join(entry.file_name());
        let file_name = entry.file_name().to_string_lossy().to_string();
        if metadata.is_symlink() {
            link_path = Some(
                storage
                    .read_link(PathBuf::from(&path).join(entry.file_name()))
                    .await
                    .inspect_err(|err| error!("Failed to list files (read_link): {err:?}"))?
                    .to_string_lossy()
                    .to_string(),
            );
        } else {
            archive = tokio::task::spawn_blocking(move || {
                let file = entry.open()?;
                let fd: OwnedFd = file.into_std().into();
                let file = std::fs::File::from(fd);
                if let Ok(Some(_)) = ArchiveCompression::detect_blocking(file) {
                    Ok::<_, io::Error>(true)
                } else {
                    Ok(false)
                }
            })
            .await
            .inspect_err(|err| error!("Failed to list files (detect): {err:?}"))
            .map_err(io::Error::from)?
            .inspect_err(|err| error!("Failed to list files (detect): {err:?}"))?;
        }

        files.push(metadata.into_fileentry(
            file_name,
            relative_path.to_string_lossy().to_string(),
            link_path,
            archive,
        ));
    }
    drop(lock);
    Ok(files)
}
