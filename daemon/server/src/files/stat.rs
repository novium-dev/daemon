use crate::Daemon;
use crate::files::{IntoFileEntry, storage_path};
use crate::storage::StorageIdentifier;
use crate::storage::archive::compression::ArchiveCompression;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::utils::path_relative;
use cap_async_std::fs::Dir;
use daemon_common::payload::files::FileEntry;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::error_file_not_found::FromFileNotFound;
use std::io;
use std::io::ErrorKind;
use std::ops::Deref;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use tracing::{debug, error};

pub(crate) async fn stat_file<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromFileNotFound,
>(
    state: &Daemon,
    id: StorageIdentifier,
    path: String,
) -> Result<FileEntry, E> {
    let StorageLockData { lock, storage, .. } =
        storage_read_timeout::<E>(&state.config, state.conn.deref(), &state.storages, id).await?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;
    let file = stat_file0::<E>(storage, path).await?;
    drop(lock);
    Ok(file)
}

#[inline(always)]
async fn stat_file0<E: From<io::Error> + FromFileNotFound>(
    storage: Dir,
    path: PathBuf,
) -> Result<FileEntry, E> {
    let metadata = storage
        .symlink_metadata(&path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound | ErrorKind::PermissionDenied => {
                debug!("Failed to stat file (metadata): {:?}", err)
            }
            _ => error!("Failed to stat file (metadata): {:?}", err),
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
                .inspect_err(|err| error!("Failed to stat file (read_link): {:?}", err))?
                .to_string_lossy()
                .to_string(),
        );
    } else {
        let file = storage
            .open(&path)
            .await
            .inspect_err(|err| error!("Failed to stat file (open): {:?}", err))?;
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
        .inspect_err(|err| error!("Failed to stat file (detect): {:?}", err))
        .map_err(io::Error::from)?
        .inspect_err(|err| error!("Failed to stat file (detect): {:?}", err))?;
    }

    let relative_path = path.join(filename.clone());
    Ok(metadata.into_fileentry(
        filename,
        relative_path.to_string_lossy().to_string(),
        link_path,
        archive,
    ))
}
