// SPDX-License-Identifier: AGPL-3.0-only
use crate::files::storage_path;
use crate::storage::StorageIdentifier;
use crate::storage::archive::compression::ArchiveCompression;
use crate::storage::archive::{TarSandboxedAppendExt as _, TarSandboxedUnpackExt as _};
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::utils::storage_parent_ino;
use crate::{Daemon, DaemonExtra, StorageNotification};
use cap_async_std::fs::{Dir, MetadataExt as _, OpenOptions};
use daemon_common::service::error_compression_unsupported::FromCompressionUnsupported;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::task::{FileOperationKind, StorageTask};
use serde_json::json;
use std::collections::HashSet;
use std::io;
use std::io::ErrorKind;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use tar::{Archive, Builder};
use tokio::sync::broadcast::Sender;
use tracing::{debug, error, warn};

/// Cancel safe
#[inline(always)]
pub async fn unpack_archive<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromCompressionUnsupported,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    path: String,
    dest: String,
) -> Result<(), E> {
    let StorageLockData {
        lock,
        storage,
        notify,
        tokens,
        ..
    } = storage_read_timeout::<E>(
        &daemon.config,
        &*daemon.conn,
        &daemon.storages,
        id,
        StorageTask::FileOperation {
            kind: FileOperationKind::ArchiveUnpack,
        },
    )
    .await?;
    let (dest_storage, dest_storage_path, dest_path) =
        storage_path(id, &storage, dest, &tokens).await?;
    let (storage, _, path) = storage_path(id, &storage, path, &tokens).await?;

    let parent_ino = storage_parent_ino(&dest_storage, &dest_path).await;

    let file = storage
        .open(path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory => debug!("Failed to unpack archive: {err:?}"),
            _ => error!("Failed to unpack archive: {err:?}"),
        })?;

    let fd: OwnedFd = file.into_std().into();
    let mut file = std::fs::File::from(fd);
    match tokio::task::spawn_blocking(move || {
        Ok::<_, io::Error>((ArchiveCompression::detect_blocking(&mut file)?, file))
    })
    .await
    .inspect_err(|err| debug!("Failed to unpack archive (detect): {err:?}"))
    .map_err(io::Error::from)?
    {
        Ok((Some(compression), file)) => {
            let archive = Archive::new(compression.create_decoder(file)?);
            archive
                .unpack_sandboxed(dest_storage_path, &dest_path, ())
                .await
                .inspect_err(|err| match err.kind() {
                    ErrorKind::InvalidData | ErrorKind::QuotaExceeded => {
                        debug!("Failed to unpack archive (unpack): {err:?}");
                    }
                    _ => error!("Failed to unpack archive (unpack): {err:?}"),
                })?;
        }
        Ok(_) => return Err(<E as FromCompressionUnsupported>::from()),
        Err(err) if err.kind() == ErrorKind::UnexpectedEof => {
            return Err(<E as FromCompressionUnsupported>::from());
        }
        Err(err) => {
            match err.kind() {
                ErrorKind::IsADirectory | ErrorKind::NotADirectory | ErrorKind::InvalidData => {
                    debug!("Failed to unpack archive (detect): {err:?}");
                }
                _ => error!("Failed to unpack archive (detect): {err:?}"),
            }
            return Err(<E as From<io::Error>>::from(err));
        }
    }

    let ino = dest_storage
        .metadata(dest_path)
        .await
        .map(|meta| meta.ino());

    if let Ok((inode, parent_inode)) = ino
        .and_then(|ino| parent_ino.map(|parent_ino| (ino, parent_ino)))
        .inspect_err(|err| {
            warn!(
                "Archive unpack was successful, but unable to trigger storage notification: {err:?}"
            );
        })
    {
        let _ = notify.send(StorageNotification::Create { parent_inode, inode })
            .inspect_err(|err| warn!("Archive unpack was successful, but unable to trigger storage notification: {err:?}"));
    }

    drop(lock);
    Ok(())
}

/// Cancel safe
pub async fn create_archive<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromCompressionUnsupported
        + Send
        + 'static,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    path: String,
    src: String,
    compression: String,
) -> Result<(), E> {
    let StorageLockData {
        lock,
        storage,
        notify,
        tokens,
        ..
    } = storage_read_timeout::<E>(
        &daemon.config,
        &*daemon.conn,
        &daemon.storages,
        id,
        StorageTask::FileOperation {
            kind: FileOperationKind::ArchiveCreate,
        },
    )
    .await?;
    let compression: ArchiveCompression = serde_json::from_value(json!({
        "type": compression
    }))
    .map_err(|_err| <E as FromCompressionUnsupported>::from())?;

    let (_, src_storage_path, src_path) = storage_path(id, &storage, src, &tokens).await?;
    let (storage, _, path) = storage_path(id, &storage, path, &tokens).await?;
    tokio::task::spawn(create_archive0::<E>(
        storage,
        src_storage_path,
        path,
        src_path,
        compression,
        notify,
    ))
    .await
    .map_err(io::Error::from)??;
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn create_archive0<E: From<io::Error>>(
    storage: Dir,
    src_storage_path: PathBuf,
    path: PathBuf,
    src: PathBuf,
    compression: ArchiveCompression,
    notify: Sender<StorageNotification>,
) -> Result<(), E> {
    let parent_ino = storage_parent_ino(&storage, &path).await;

    let file = storage
        .open_with(
            &path,
            OpenOptions::new().create(true).truncate(true).write(true),
        )
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::QuotaExceeded => debug!("Failed to create archive (open): {err:?}"),
            _ => error!("Failed to create archive (open): {err:?}"),
        })?;

    let ino = storage.metadata(path).await.map(|meta| meta.ino());

    let fd: OwnedFd = file.into_std().into();
    let file = std::fs::File::from(fd);
    let archive = Builder::new(
        compression
            .create_encoder(file)
            .inspect_err(|err| error!("Failed to create archive (create_encoder): {err:?}"))?,
    );
    let (archive, ()) = archive
        .append_dir_all_sandboxed(src_storage_path, "", src, HashSet::new(), (), true, false)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::QuotaExceeded => debug!("Failed to create archive (create): {err:?}"),
            _ => error!("Failed to create archive (create): {err:?}"),
        })?;

    tokio::task::spawn_blocking(move || {
        use std::io::Write as _;
        archive.into_inner()?.finish()?.flush()
    })
    .await
    .inspect_err(|err| error!("Failed to create archive (flush): {err:?}"))
    .map_err(io::Error::from)?
    .inspect_err(|err| match err.kind() {
        ErrorKind::QuotaExceeded => debug!("Failed to create archive (flush): {err:?}"),
        _ => error!("Failed to create archive (flush): {err:?}"),
    })?;

    let _ = notify
        .send(StorageNotification::Update {
            parent_inode: parent_ino
                .inspect_err(|err| {
                    warn!(
                        "Archive creation was successful, but unable to trigger storage notification: {err:?}"
                    );
                })
                .ok(),
            inode: ino
                .inspect_err(|err| {
                    warn!(
                        "Archive creation was successful, but unable to trigger storage notification: {err:?}"
                    );
                })
                .ok(),
        })
        .inspect_err(|err| {
            warn!("Archive creation was successful, but unable to trigger storage notification: {err:?}");
        });

    Ok(())
}
