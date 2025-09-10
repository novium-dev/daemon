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
use crate::files::storage_path;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::{Daemon, DaemonExtra};
use axum::http;
use axum::http::{HeaderMap, HeaderValue};
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::error_file_limit_reached::FromFileLimitReached;
use daemon_common::service::{DataStream, FileResponse};
use std::io;
use std::io::ErrorKind;
use std::os::fd::OwnedFd;
use tracing::{debug, error};

/// Cancel safe
pub async fn download_file<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromFileLimitReached,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    path: String,
    limit: Option<u64>,
) -> Result<FileResponse, E> {
    let StorageLockData {
        lock,
        storage,
        tokens,
        ..
    } = storage_read_timeout::<E>(&daemon.config, &*daemon.conn, &daemon.storages, id).await?;
    let (storage, _, path) = storage_path(id, &storage, path, &tokens).await?;
    let fd: OwnedFd = storage
        .open(&path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound | ErrorKind::PermissionDenied | ErrorKind::IsADirectory => {
                debug!("Failed to download file: {err:?}");
            }
            _ => error!("Failed to download file: {err:?}"),
        })?
        .into_std()
        .into();
    let metadata = storage
        .metadata(&path)
        .await
        .inspect_err(|err| error!("Failed to download file (metadata): {err:?}"))?;
    let length = metadata.len();
    if let Some(limit) = limit
        && length > limit
    {
        return Err(<E as FromFileLimitReached>::from());
    }

    let mut headers =
        HeaderMap::from_iter([(http::header::CONTENT_LENGTH, HeaderValue::from(length))]);
    if let Some(filename) = path.file_name().map(|str| str.to_string_lossy())
        && let Ok(filename) = format!("attachment; filename=\"{filename}\"").parse()
    {
        headers.insert(http::header::CONTENT_DISPOSITION, filename);
    }
    let fd = tokio::fs::File::from_std(std::fs::File::from(fd));
    drop(lock);
    Ok(FileResponse(Some(headers), DataStream::by_read(fd)))
}
