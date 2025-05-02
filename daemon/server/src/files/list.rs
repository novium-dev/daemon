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
use std::io;
use std::io::ErrorKind;
use std::ops::Deref;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use tracing::{debug, error};

pub(crate) async fn list_files<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure,
>(
    state: &Daemon,
    id: StorageIdentifier,
    path: String,
) -> Result<Vec<FileEntry>, E> {
    let StorageLockData { lock, storage, .. } =
        storage_read_timeout::<E>(&state.config, state.conn.deref(), &state.storages, id).await?;
    let path = path_relative(path);
    let (storage, _) = storage_path(id, &storage).await?;
    let files = list_files0::<E>(storage, path).await?;
    drop(lock);
    Ok(files)
}

#[inline(always)]
async fn list_files0<E: From<io::Error>>(storage: Dir, path: PathBuf) -> Result<Vec<FileEntry>, E> {
    let read_dir = storage
        .read_dir(&path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound | ErrorKind::PermissionDenied | ErrorKind::NotADirectory => {
                debug!("Failed to list files (read_dir): {:?}", err)
            }
            _ => error!("Failed to list files (read_dir): {:?}", err),
        })?;
    let mut vec = Vec::new();
    for entry in read_dir {
        let entry = entry.inspect_err(|err| error!("Failed to list files (entry): {:?}", err))?;
        let metadata = entry
            .metadata()
            .inspect_err(|err| error!("Failed to list files (metadata): {:?}", err))?;
        let mut link_path = None;
        let mut archive = false;

        let relative_path = path.join(entry.file_name());
        let file_name = entry.file_name().to_string_lossy().to_string();
        if metadata.is_symlink() {
            link_path = Some(
                storage
                    .read_link(PathBuf::from(&path).join(entry.file_name()))
                    .await
                    .inspect_err(|err| error!("Failed to list files (read_link): {:?}", err))?
                    .to_string_lossy()
                    .to_string(),
            );
        } else {
            archive = tokio::task::spawn_blocking(move || {
                let file = entry.open()?;
                let fd: OwnedFd = file.into_std().into();
                let file = std::fs::File::from(fd);
                if let Ok(Some(_)) = ArchiveCompression::detect_blocking(file) {
                    Ok::<_, io::Error>(true)
                } else {
                    Ok(false)
                }
            })
            .await
            .inspect_err(|err| error!("Failed to list files (detect): {:?}", err))
            .map_err(io::Error::from)?
            .inspect_err(|err| error!("Failed to list files (detect): {:?}", err))?;
        }

        vec.push(metadata.into_fileentry(
            file_name,
            relative_path.to_string_lossy().to_string(),
            link_path,
            archive,
        ));
    }

    Ok(vec)
}
