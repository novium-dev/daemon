// SPDX-License-Identifier: AGPL-3.0-only
use crate::storage::StorageIdentifier;
use crate::storage::archive::TarSandboxedAppendExt as _;
use crate::storage::archive::compression::ArchiveCompression;
use crate::storage::archive::format::ArchiveFormat;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::{Daemon, DaemonExtra};
use axum_extra::response::Attachment;
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
use std::io::Write as _;
use std::path::{Path, PathBuf};
use tar::Builder;
use tokio_util::io::SyncIoBridge;
use tracing::debug;

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

    let attachment = backup
        .backend
        .download(id, Path::new(&backup.path), backup_id, backup_exclude)
        .await?;
    drop(lock);

    Ok(FileResponse::Axum(attachment))
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
            .filename(format!("{}.tar{}", backup_filename(id), compression.ext())),
    ))
}

pub fn backup_filename(id: StorageIdentifier) -> String {
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
pub async fn stream_realtime1(
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
