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
use crate::files::{IntoFileEntry as _, storage_path};
use crate::storage::StorageIdentifier;
use crate::storage::archive::compression::ArchiveCompression;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::{Daemon, DaemonExtra};
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::FileEntry;
use daemon_common::service::files::error_file_not_found::FromFileNotFound;
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
    } = storage_read_timeout::<E>(&daemon.config, &*daemon.conn, &daemon.storages, id).await?;
    let (storage, _, path) = storage_path(id, &storage, path, &tokens).await?;
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
            if let Ok(Some(_)) = ArchiveCompression::detect_blocking(file) {
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
    drop(lock);
    Ok(metadata.into_fileentry(
        filename,
        relative_path.to_string_lossy().to_string(),
        link_path,
        archive,
    ))
}
