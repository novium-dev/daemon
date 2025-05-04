use crate::Daemon;
use crate::files::storage_path;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::utils::path_relative;
use axum_extra::TypedHeader;
use axum_extra::headers::ContentLength;
use cap_async_std::fs::{Dir, OpenOptions};
use daemon_common::DataStream;
use daemon_common::payload::server::ServerFileWrite;
use daemon_common::service::error_http_request_failed::FromHttpRequestFailed;
use daemon_common::service::error_http_request_not_success::FromHttpRequestNotSuccess;
use daemon_common::service::error_http_request_read_failed::FromHttpRequestReadFailed;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::error_storage_quota_reached::FromStorageQuotaReached;
use std::io;
use std::io::ErrorKind;
use std::ops::Deref;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::task::JoinError;
use tracing::{debug, error, warn};
use url::Url;

/// Cancel safe
pub(crate) async fn write_file<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure,
>(
    state: &Daemon,
    id: StorageIdentifier,
    path: String,
    length: Option<TypedHeader<ContentLength>>,
    stream: DataStream,
) -> Result<(), E> {
    let _length = length.map(|header| header.0.0);

    let StorageLockData { lock, storage, .. } =
        storage_read_timeout::<E>(&state.config, state.conn.deref(), &state.storages, id).await?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;
    let mut reader = stream.reader();
    let fd: OwnedFd = storage
        .open_with(
            path,
            OpenOptions::new().create(true).truncate(true).write(true),
        )
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::QuotaExceeded => debug!("Failed to write file (open): {:?}", err),
            _ => error!("Failed to write file (open): {:?}", err),
        })?
        .into_std()
        .into();
    let mut writer = BufWriter::new(tokio::fs::File::from_std(std::fs::File::from(fd)));
    tokio::io::copy(&mut reader, &mut writer)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::QuotaExceeded => debug!("Failed to write file: {:?}", err),
            _ => error!("Failed to write file: {:?}", err),
        })?;
    drop(lock);
    Ok(())
}

/// Cancel safe
pub(crate) async fn write_file_by_template<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromStorageQuotaReached
        + Send
        + 'static,
>(
    state: &Daemon,
    id: StorageIdentifier,
    path: String,
    template: ServerFileWrite,
) -> Result<(), E> {
    let StorageLockData { lock, storage, .. } =
        storage_read_timeout::<E>(&state.config, state.conn.deref(), &state.storages, id).await?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;

    let fd: OwnedFd = storage
        .open_with(path, OpenOptions::new().create(true).read(true).write(true))
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::QuotaExceeded => {
                debug!("Failed to write file by template (open): {:?}", err)
            }
            _ => error!("Failed to write file by template (open): {:?}", err),
        })?
        .into_std()
        .into();
    let file = tokio::fs::File::from_std(std::fs::File::from(fd));
    tokio::task::spawn(async move { template.apply::<E>(file).await })
        .await
        .map_err(<io::Error as From<JoinError>>::from)??;

    drop(lock);
    Ok(())
}

/// Cancel safe
pub(crate) async fn write_file_by_url<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromHttpRequestFailed
        + FromHttpRequestNotSuccess
        + FromHttpRequestReadFailed
        + Send
        + 'static,
>(
    state: &Daemon,
    id: StorageIdentifier,
    path: String,
    url: Url,
) -> Result<(), E> {
    let StorageLockData { lock, storage, .. } =
        storage_read_timeout::<E>(&state.config, state.conn.deref(), &state.storages, id).await?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;
    tokio::task::spawn(write_file_by_url0::<E>(state.clone(), storage, path, url))
        .await
        .map_err(<io::Error as From<JoinError>>::from)??;
    drop(lock);
    Ok(())
}

async fn write_file_by_url0<
    E: From<io::Error> + FromHttpRequestFailed + FromHttpRequestNotSuccess + FromHttpRequestReadFailed,
>(
    state: Daemon,
    storage: Dir,
    path: PathBuf,
    url: Url,
) -> Result<(), E> {
    let mut response = state
        .client
        .get(url.clone())
        .send()
        .await
        .inspect_err(|err| warn!("Failed to write file by url {} (request): {:?}", url, err))
        .map_err(|_err| <E as FromHttpRequestFailed>::from())?
        .error_for_status()
        .inspect_err(|err| warn!("Failed to write file by url {} (status): {:?}", url, err))
        .map_err(|_err| <E as FromHttpRequestNotSuccess>::from())?;

    let fd: OwnedFd = storage
        .open_with(
            path,
            OpenOptions::new().create(true).truncate(true).write(true),
        )
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::QuotaExceeded => debug!("Failed to write file by url (open): {:?}", err),
            _ => error!("Failed to write file by url (open): {:?}", err),
        })?
        .into_std()
        .into();
    let mut writer = BufWriter::new(tokio::fs::File::from_std(std::fs::File::from(fd)));
    while let Some(chunk) = response
        .chunk()
        .await
        .inspect_err(|err| warn!("Failed to read by url: {:?}", err))
        .map_err(|_err| <E as FromHttpRequestReadFailed>::from())?
    {
        writer
            .write(&chunk)
            .await
            .inspect_err(|err| match err.kind() {
                ErrorKind::QuotaExceeded => debug!("Failed to write file by url: {:?}", err),
                _ => error!("Failed to write file by url: {:?}", err),
            })?;
    }
    writer.flush().await?;
    Ok(())
}
