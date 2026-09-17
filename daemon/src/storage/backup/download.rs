// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::BackupEntry;
use crate::storage::StorageIdentifier;
use crate::storage::archive::TarSandboxedAppendExt as _;
use crate::storage::archive::compression::ArchiveCompression;
use crate::storage::archive::format::ArchiveFormat;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::{Daemon, DaemonExtra};
use axum_extra::headers::ContentLength;
use axum_extra::response::Attachment;
use chrono::{TimeZone as _, Utc};
use daemon_common::service::backup::error_backup_storage_location_not_found::FromBackupStorageLocationNotFound;
use daemon_common::service::error_compression_unsupported::FromCompressionUnsupported;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::task::{BackupLocation, StorageTask};
use daemon_common::service::{DataStream, FileResponse};
use serde_json::json;
use std::collections::HashSet;
use std::io;
use std::io::{ErrorKind, Write as _};
use std::path::PathBuf;
use tar::Builder;
use tokio::fs::File;
use tokio_util::io::SyncIoBridge;
use tracing::{debug, error};

/// Cancel safe
pub async fn backup_to_stream<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromBackupStorageLocationNotFound,
>(
    state: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    backup_id: i64,
    backup_storage: &String,
) -> Result<FileResponse, E> {
    let backup = state
        .config
        .backup
        .get(backup_storage)
        .ok_or_else(<E as FromBackupStorageLocationNotFound>::from)
        .inspect_err(|_err| debug!("Backup storage does not exist: {backup_storage}"))?;
    let StorageLockData {
        lock,
        #[cfg(feature = "btrfs")]
        backup_exclude,
        ..
    } = storage_read_timeout::<E>(
        &state.config,
        &*state.conn,
        &state.storages,
        id,
        StorageTask::BackupDownload {
            location: BackupLocation {
                storage: backup_storage.clone(),
                id: backup_id,
            },
        },
    )
    .await?;

    let attachment = backup_to_stream0::<E>(
        id,
        backup_id,
        backup,
        #[cfg(feature = "btrfs")]
        backup_exclude,
    )
    .await?;
    drop(lock);

    Ok(FileResponse::Axum(attachment))
}

/// Cancel safe
#[inline(always)]
async fn backup_to_stream0<E: From<io::Error>>(
    id: StorageIdentifier,
    backup_id: i64,
    backup: &BackupEntry,
    #[cfg(feature = "btrfs")] exclude: HashSet<String>,
) -> Result<Attachment<DataStream>, E> {
    let target_dir = PathBuf::from(backup.path()).join(id.as_pathbuf());
    match backup {
        BackupEntry::Archive { compression, .. } => {
            let target = target_dir.join(format!("{backup_id}.tar{}", compression.ext()));

            let file = File::open(target).await.inspect_err(|err| {
                if err.kind() == ErrorKind::NotFound {
                    debug!("Failed to create a backup stream (open): {err:?}");
                } else {
                    error!("Failed to create a backup stream (open): {err:?}");
                }
            })?;
            let length = file
                .metadata()
                .await
                .inspect_err(|err| error!("Failed to create a backup stream (metadata): {err:?}"))?
                .len();

            Ok(
                Attachment::new(DataStream::by_read(file, Some(ContentLength(length))))
                    .content_type(ArchiveFormat::Tar(*compression).content_type())
                    .filename(format!(
                        "{}_{}.tar{}",
                        filename(id),
                        Utc.timestamp_millis_opt(backup_id).unwrap().to_rfc3339(),
                        compression.ext()
                    )),
            )
        }
        #[cfg(feature = "btrfs")]
        BackupEntry::Btrfs {
            jit_compression, ..
        } => {
            let compression = *jit_compression;

            let target = target_dir.join(backup_id.to_string());
            Ok(
                Attachment::new(stream_realtime1(compression, target, exclude).await)
                    .content_type(ArchiveFormat::Tar(compression).content_type())
                    .filename(format!(
                        "{}_{}.tar{}",
                        filename(id),
                        Utc.timestamp_millis_opt(backup_id).unwrap().to_rfc3339(),
                        compression.ext()
                    )),
            )
        }
    }
}

/// Cancel safe
pub async fn stream_realtime<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromCompressionUnsupported,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    compression: String,
) -> Result<FileResponse, E> {
    let StorageLockData {
        path,
        lock,
        backup_exclude,
        ..
    } = storage_read_timeout::<E>(
        &daemon.config,
        &*daemon.conn,
        &daemon.storages,
        id,
        StorageTask::RealtimeDownload,
    )
    .await?;
    let compression: ArchiveCompression = serde_json::from_value(json!({
        "type": compression
    }))
    .map_err(|_err| <E as FromCompressionUnsupported>::from())?;

    let data = stream_realtime0(path, backup_exclude, compression).await;
    drop(lock);

    Ok(FileResponse::Axum(
        Attachment::new(data)
            .content_type(ArchiveFormat::Tar(compression).content_type())
            .filename(format!("{}.tar{}", filename(id), compression.ext())),
    ))
}

fn filename(id: StorageIdentifier) -> String {
    match id {
        StorageIdentifier::Primary(id) => format!("server_{id}"),
    }
}

/// Cancel safe
#[inline(always)]
fn stream_realtime0(
    path: String,
    exclude: HashSet<String>,
    compression: ArchiveCompression,
) -> impl Future<Output = DataStream> + Send {
    stream_realtime1(compression, PathBuf::from(path), exclude)
}

/// Cancel safe
#[expect(
    clippy::unused_async,
    reason = "ensure it only gets called in an async context"
)]
async fn stream_realtime1(
    compression: ArchiveCompression,
    target: PathBuf,
    exclude: HashSet<String>,
) -> DataStream {
    let (sender, recv) = tokio::io::duplex(1024 * 1024);
    tokio::task::spawn_blocking(move || {
        let encoder = compression.create_encoder(SyncIoBridge::new(sender))?;
        let mut archive = Builder::new(encoder);
        archive.follow_symlinks(false);
        let (archive, ()) = archive.append_dir_all_sandboxed_blocking(
            &target,
            "",
            "/",
            exclude,
            (),
            true,
            false,
        )?;
        archive.into_inner()?.finish()?.flush()?;
        Ok::<(), io::Error>(())
    });
    DataStream::by_read(recv, None)
}
