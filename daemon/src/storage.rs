pub(crate) mod backup;

use crate::config::StorageEntry;
use crate::utils;
use std::io;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use tokio::fs;
use tokio::fs::File;
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
                if let Some(parent) = dir.parent() {
                    fs::create_dir_all(parent).await.inspect_err(|err| {
                        error!("Unable to create parents of storage directory: {:?}", err)
                    })?;
                }
                tokio::task::spawn_blocking(move || {
                    libbtrfs::subvolume::new(&dir.to_string_lossy())?;
                    Ok::<(), io::Error>(())
                })
                .await
                .map_err(|err| io::Error::new(ErrorKind::Other, err))?
                .inspect_err(|err| {
                    error!("Unable to create subvolume of storage directory: {:?}", err)
                })?;
            }
        }
    }

    #[cfg(feature = "btrfs")]
    if let StorageEntry::Btrfs { quota: true, .. } = &storage {
        use libbtrfs::btrfs::*;
        let args = btrfs_ioctl_qgroup_limit_args {
            qgroupid: 0,
            lim: btrfs_qgroup_limit {
                flags: BTRFS_QGROUP_LIMIT_RFER_CMPR
                    | BTRFS_QGROUP_LIMIT_EXCL_CMPR
                    | BTRFS_QGROUP_LIMIT_MAX_RFER,
                max_rfer: disk.map(utils::from_mib).unwrap_or(u64::MAX), // This is the value for no limit

                rsv_rfer: 0,
                rsv_excl: 0,
                max_excl: 0,
            },
        };
        let file = File::open(&dir).await?;

        let ret = unsafe { libc::ioctl(file.as_raw_fd(), BTRFS_IOC_QGROUP_LIMIT, &args) };
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
    }
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
            return tokio::task::spawn_blocking(move || {
                libbtrfs::subvolume::remove(&dir.to_string_lossy())?;
                Ok(())
            })
            .await
            .inspect_err(|err| error!("Unable to remove subvolume of storage directory: {:?}", err))
            .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
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
