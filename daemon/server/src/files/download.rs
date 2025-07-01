use crate::Daemon;
use crate::files::storage_path;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::utils::path_relative;
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
    daemon: &Daemon,
    id: StorageIdentifier,
    path: String,
    limit: Option<u64>,
) -> Result<FileResponse, E> {
    let StorageLockData { lock, storage, .. } =
        storage_read_timeout::<E>(&daemon.config, &*daemon.conn, &daemon.storages, id).await?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;
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
    if let Some(limit) = limit {
        if length > limit {
            return Err(<E as FromFileLimitReached>::from());
        }
    }

    let mut headers =
        HeaderMap::from_iter([(http::header::CONTENT_LENGTH, HeaderValue::from(length))]);
    if let Some(filename) = path.file_name().map(|str| str.to_string_lossy()) {
        headers.insert(
            http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", filename)
                .parse()
                .unwrap(),
        );
    }
    let fd = tokio::fs::File::from_std(std::fs::File::from(fd));
    drop(lock);
    Ok(FileResponse(Some(headers), DataStream::by_read(fd)))
}
