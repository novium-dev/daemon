#[cfg(feature = "backups")]
pub(crate) mod backup;

use crate::config::StorageEntry;
use std::io;
use std::path::PathBuf;
use tokio::fs;
use tracing::debug;

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub(crate) enum StorageIdentifier {
    Primary(u32),
}

impl StorageIdentifier {
    pub(crate) fn as_pathbuf(&self) -> PathBuf {
        match self {
            StorageIdentifier::Primary(id) => PathBuf::from(format!("primary/{}", id)),
        }
    }

    #[allow(unused)]
    pub(crate) fn as_vec(&self) -> Vec<String> {
        match self {
            StorageIdentifier::Primary(id) => vec!["primary".to_owned(), id.to_string()],
        }
    }
}

pub(crate) async fn ensure_exists(
    id: StorageIdentifier,
    storage: &StorageEntry,
    _disk: Option<u64>,
) -> Result<String, io::Error> {
    let base = PathBuf::from(storage.path());
    let dir = base.join(id.as_pathbuf());

    if !dir
        .try_exists()
        .inspect_err(|err| debug!("Unable to check existence of storage directory: {:?}", err))?
    {
        match &storage {
            StorageEntry::Directory { .. } => {
                fs::create_dir_all(&dir)
                    .await
                    .inspect_err(|err| debug!("Unable to create storage directory: {:?}", err))?;
            }
            #[cfg(feature = "btrfs")]
            StorageEntry::Btrfs { .. } => {
                use std::io::ErrorKind;
                let dir = dir.clone();
                if let Some(parent) = dir.parent() {
                    fs::create_dir_all(parent).await.inspect_err(|err| {
                        debug!("Unable to create parents of storage directory: {:?}", err)
                    })?;
                }
                // TODO: Quota
                tokio::task::spawn_blocking(move || {
                    libbtrfs::subvolume::new(&dir.to_string_lossy())?;
                    Ok::<(), io::Error>(())
                })
                .await
                .map_err(|err| io::Error::new(ErrorKind::Other, err))?
                .inspect_err(|err| {
                    debug!("Unable to create subvolume of storage directory: {:?}", err)
                })?;
            }
        }
    }

    #[cfg(feature = "btrfs")]
    if let StorageEntry::Btrfs { .. } = &storage {
        // TODO: Update quota
    }
    Ok(fs::canonicalize(dir)
        .await
        .inspect_err(|err| debug!("Unable to canonicalize storage directory: {:?}", err))?
        .to_string_lossy()
        .to_string())
}

pub(crate) async fn delete(storage: &StorageEntry, id: StorageIdentifier) -> Result<(), io::Error> {
    let base = PathBuf::from(storage.path());
    let dir = base.join(id.as_pathbuf());

    #[cfg(feature = "btrfs")]
    if let StorageEntry::Btrfs { .. } = storage {
        use std::io::ErrorKind;
        return tokio::task::spawn_blocking(move || {
            libbtrfs::subvolume::remove(&dir.to_string_lossy())?;
            Ok(())
        })
        .await
        .inspect_err(|err| debug!("Unable to remove subvolume of storage directory: {:?}", err))
        .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
    }

    if dir
        .try_exists()
        .inspect_err(|err| debug!("Unable to check if storage directory exists: {:?}", err))?
    {
        fs::remove_dir_all(&dir)
            .await
            .inspect_err(|err| debug!("Unable to remove storage directory: {:?}", err))?;
    }
    Ok(())
}
