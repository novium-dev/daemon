pub(crate) mod archive;
pub(crate) mod backup;
#[cfg(feature = "btrfs")]
pub(crate) mod btrfs;
pub(crate) mod sync;
pub(crate) mod worker;

use crate::config::StorageEntry;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use tokio::fs;
use tracing::error;

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
    value: Arc<(AtomicI64, AtomicI64)>,
    disk: Option<u64>,
) -> Result<String, io::Error> {
    let base = PathBuf::from(storage.path());
    let dir = base.join(id.as_pathbuf());

    if !dir
        .try_exists()
        .inspect_err(|err| error!("Unable to check existence of storage directory: {:?}", err))?
    {
        match &storage {
            StorageEntry::Directory { .. } => {
                fs::create_dir_all(&dir)
                    .await
                    .inspect_err(|err| error!("Unable to create storage directory: {:?}", err))?;
            }
            #[cfg(feature = "btrfs")]
            StorageEntry::Btrfs { .. } => {
                use std::io::ErrorKind;
                let dir = dir.clone();
                let Some(parent) = dir.parent() else {
                    error!("Got path without parent: {:?}", dir.to_string_lossy());
                    return Err(io::Error::new(ErrorKind::Other, "Invalid path: No parent"));
                };
                fs::create_dir_all(parent).await.inspect_err(|err| {
                    error!("Unable to create parents of storage directory: {:?}", err)
                })?;
                btrfs::new_subvolume(parent, dir.file_name().unwrap().to_str().unwrap())
                    .await
                    .inspect_err(|err| {
                        error!("Unable to create subvolume of storage directory: {:?}", err)
                    })?;
            }
        }
    }

    #[cfg(feature = "btrfs")]
    if let StorageEntry::Btrfs { quota: true, .. } = &storage {
        btrfs::apply_qgroup_limit(&dir, disk).await?;
    }
    value
        .1
        .store(disk.map(|i| i as i64).unwrap_or(-1), Ordering::Relaxed);

    Ok(fs::canonicalize(dir)
        .await
        .inspect_err(|err| error!("Unable to canonicalize storage directory: {:?}", err))?
        .to_string_lossy()
        .to_string())
}

pub(crate) async fn delete(storage: &StorageEntry, id: StorageIdentifier) -> Result<(), io::Error> {
    let base = PathBuf::from(storage.path());
    let dir = base.join(id.as_pathbuf());

    #[cfg(feature = "btrfs")]
    if dir
        .try_exists()
        .inspect_err(|err| error!("Unable to check if storage directory exists: {:?}", err))?
    {
        if let StorageEntry::Btrfs { .. } = storage {
            use std::io::ErrorKind;
            let Some(parent) = dir.parent() else {
                error!("Got path without parent: {:?}", dir.to_string_lossy());
                return Err(io::Error::new(ErrorKind::Other, "Invalid path: No parent"));
            };
            btrfs::remove_subvolume(parent, dir.file_name().unwrap().to_str().unwrap())
                .await
                .inspect_err(|err| {
                    error!("Unable to remove subvolume of storage directory: {:?}", err)
                })?;
        }
    }
    if dir
        .try_exists()
        .inspect_err(|err| error!("Unable to check if storage directory exists: {:?}", err))?
    {
        fs::remove_dir_all(&dir)
            .await
            .inspect_err(|err| error!("Unable to remove storage directory: {:?}", err))?;
    }
    Ok(())
}
