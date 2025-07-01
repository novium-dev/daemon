pub mod archive;
pub mod backup;
#[cfg(feature = "btrfs")]
pub mod btrfs;
pub mod sync;
pub mod worker;

use crate::config::StorageEntry;
use core::sync::atomic::{AtomicI64, Ordering};
use std::io;
use std::path::PathBuf;
use tokio::fs;
use tracing::error;

#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub enum StorageIdentifier {
    Primary(u32),
}

impl StorageIdentifier {
    pub fn as_pathbuf(self) -> PathBuf {
        match self {
            Self::Primary(id) => PathBuf::from(format!("primary/{id}")),
        }
    }
}

/// Cancel safe (possible subvolume without quota, should correct itself after next storage access)
pub async fn prepare(
    id: StorageIdentifier,
    storage: &StorageEntry,
    value: &(AtomicI64, AtomicI64),
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
                let dir = dir.clone();
                let Some(parent) = dir.parent() else {
                    error!("Got path without parent: {:?}", dir.to_string_lossy());
                    return Err(io::Error::other("Invalid path: No parent"));
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
    value.1.store(
        disk.filter(|value| *value <= i64::MAX.unsigned_abs())
            .map_or(-1, u64::cast_signed),
        Ordering::Relaxed,
    );

    Ok(fs::canonicalize(dir)
        .await
        .inspect_err(|err| error!("Unable to canonicalize storage directory: {:?}", err))?
        .to_string_lossy()
        .to_string())
}

/// Cancel safe
pub async fn delete(storage: &StorageEntry, id: StorageIdentifier) -> Result<(), io::Error> {
    let base = PathBuf::from(storage.path());
    let dir = base.join(id.as_pathbuf());

    #[cfg(feature = "btrfs")]
    if dir
        .try_exists()
        .inspect_err(|err| error!("Unable to check if storage directory exists: {:?}", err))?
    {
        if let StorageEntry::Btrfs { .. } = storage {
            let Some(parent) = dir.parent() else {
                error!("Got path without parent: {:?}", dir.to_string_lossy());
                return Err(io::Error::other("Invalid path: No parent"));
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
        tokio::task::spawn(fs::remove_dir_all(dir))
            .await
            .inspect_err(|err| error!("Unable to remove storage directory: {:?}", err))
            .map_err(io::Error::from)?
            .inspect_err(|err| error!("Unable to remove storage directory: {:?}", err))?;
    }
    Ok(())
}
