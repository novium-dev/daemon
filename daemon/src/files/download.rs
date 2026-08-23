// SPDX-License-Identifier: AGPL-3.0-only
use crate::files::storage_path;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::{Daemon, DaemonExtra};
use axum_extra::headers::ContentLength;
use axum_extra::response::Attachment;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::error_file_limit_reached::FromFileLimitReached;
use daemon_common::service::files::task::{FileOperationKind, StorageTask};
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
    } = storage_read_timeout::<E>(
        &daemon.config,
        &*daemon.conn,
        &daemon.storages,
        id,
        StorageTask::FileOperation {
            kind: FileOperationKind::Download,
        },
    )
    .await?;
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

    let fd = tokio::fs::File::from_std(std::fs::File::from(fd));
    drop(lock);
    let mut attachment = Attachment::new(DataStream::by_read(fd, Some(ContentLength(length))));
    if let Some(filename) = path.file_name()
        && let Some(filename) = filename.to_str()
    {
        attachment = attachment.filename(filename);
    }
    Ok(FileResponse::Axum(attachment))
}
