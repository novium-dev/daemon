use std::collections::HashSet;
use std::io;
use std::io::ErrorKind;
use std::ops::Deref;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use cap_async_std::fs::{Dir, OpenOptions};
use serde_json::json;
use tar::{Archive, Builder};
use tracing::{debug, error};
use daemon_common::service::error_compression_unsupported::FromCompressionUnsupported;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use crate::Daemon;
use crate::files::storage_path;
use crate::storage::archive::compression::ArchiveCompression;
use crate::storage::archive::{TarSandboxedAppendExtension, TarSandboxedUnpackExtension};
use crate::storage::StorageIdentifier;
use crate::storage::sync::{storage_read_timeout, StorageLockData};
use crate::utils::path_relative;

pub(crate) async fn unpack_archive<E: From<io::Error> + From<sqlx::Error> + FromStorageNotFound + FromStorageMutexLocked + FromStorageFailure + FromCompressionUnsupported>(
    state: &Daemon,
    id: StorageIdentifier,
    path: String,
    dest: String,
) -> Result<(), E> {
    let StorageLockData { lock, storage, .. } = storage_read_timeout::<E>(
        &state.config,
        state.conn.deref(),
        &state.storages,
        id,
    )
    .await?;
    let path = path_relative(path);
    let (storage, storage_path) = storage_path(id, &storage).await?;
    unpack_archive0::<E>(storage, storage_path, path, dest)
        .await?;
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn unpack_archive0<E: From<io::Error> + FromCompressionUnsupported>(
    storage: Dir,
    storage_path: PathBuf,
    path: PathBuf,
    dest: String,
) -> Result<(), E> {
    let file = storage.open(path).await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::NotADirectory => debug!("Failed to unpack archive: {:?}", err),
            _ => error!("Failed to unpack archive: {:?}", err),
        })?;

    let fd: OwnedFd = file.into_std().into();
    let mut file = std::fs::File::from(fd);
    match tokio::task::spawn_blocking(move || {
        Ok::<_, io::Error>((ArchiveCompression::detect_blocking(&mut file)?, file))
    })
    .await.inspect_err(|err| debug!("Failed to unpack archive (detect): {:?}", err)).map_err(io::Error::from)?
    {
        Ok((Some(compression), file)) => {
            let archive = Archive::new(compression.create_decoder(file)?);
            archive.unpack_sandboxed(storage_path, dest, ()).await
                .inspect_err(|err| match err.kind() {
                    ErrorKind::InvalidData | ErrorKind::QuotaExceeded => debug!("Failed to unpack archive (unpack): {:?}", err),
                    _ => error!("Failed to unpack archive (unpack): {:?}", err)
                })?;

            Ok(())
        }
        Ok(_) => Err(<E as FromCompressionUnsupported>::from()),
        Err(err) if err.kind() == ErrorKind::UnexpectedEof => Err(<E as FromCompressionUnsupported>::from()),
        Err(err) => {
            match err.kind() {
                ErrorKind::IsADirectory
                | ErrorKind::NotADirectory
                | ErrorKind::InvalidData => debug!("Failed to unpack archive (detect): {:?}", err),
                _ => error!("Failed to unpack archive (detect): {:?}", err),
            }
            Err(<E as From<io::Error>>::from(err))
        },
    }
}

pub(crate) async fn create_archive<E: From<io::Error> + From<sqlx::Error> + FromStorageNotFound + FromStorageMutexLocked + FromStorageFailure + FromCompressionUnsupported>(
    state: &Daemon,
    id: StorageIdentifier,
    path: String,
    src: String,
    compression: String,
) -> Result<(), E> {
    let StorageLockData { lock, storage, .. } = storage_read_timeout::<E>(
        &state.config,
        state.conn.deref(),
        &state.storages,
        id,
    )
    .await?;
    let path = path_relative(path);

    let compression: ArchiveCompression = serde_json::from_value(json!({
        "type": compression
    }))
    .map_err(|_err| <E as FromCompressionUnsupported>::from())?;

    let (storage, storage_path) = storage_path(id, &storage).await?;
    create_archive0::<E>(storage, storage_path, path, src, compression)
        .await?;
    drop(lock);
    Ok(())
}

#[inline(always)]
async fn create_archive0<E: From<io::Error>>(
    storage: Dir,
    storage_path: PathBuf,
    path: PathBuf,
    src: String,
    compression: ArchiveCompression,
) -> Result<(), E> {
    let file = storage
        .open_with(
            path,
            OpenOptions::new().create(true).truncate(true).write(true),
        )
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::IsADirectory
            | ErrorKind::QuotaExceeded => debug!("Failed to create archive (open): {:?}", err),
            _ => error!("Failed to create archive (open): {:?}", err),
        })?;

    let fd: OwnedFd = file.into_std().into();
    let file = std::fs::File::from(fd);
    let archive = Builder::new(compression.create_encoder(file).inspect_err(|err| error!("Failed to create archive (create_encoder): {:?}", err))?);
    let (archive, _) = archive
        .append_dir_all_sandboxed(storage_path, "", src, HashSet::new(), (), true)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::QuotaExceeded => debug!("Failed to create archive (create): {:?}", err),
            _ => error!("Failed to create archive (create): {:?}", err),
        })?;

    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        archive.into_inner().unwrap().finish()?.flush()
    })
    .await
        .inspect_err(|err| error!("Failed to create archive (flush): {:?}", err))
        .map_err(io::Error::from)?
        .inspect_err(|err| match err.kind() {
            ErrorKind::QuotaExceeded => debug!("Failed to create archive (flush): {:?}", err),
            _ => error!("Failed to create archive (flush): {:?}", err),
        })?;
    Ok(())
}

