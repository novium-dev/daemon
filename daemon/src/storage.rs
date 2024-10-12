#[cfg(feature = "backups")]
pub(crate) mod backup;

use crate::config::{DaemonConfig, StorageMethod};
use std::io;
use std::path::PathBuf;
use tokio::fs;

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
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
    config: &DaemonConfig,
    id: StorageIdentifier,
    _disk: Option<u64>,
) -> Result<String, io::Error> {
    let base = PathBuf::from(&config.storage.path);
    let dir = base.join(id.as_pathbuf());

    if !dir.try_exists()? {
        match config.storage.method {
            StorageMethod::Directory => {
                fs::create_dir_all(&dir).await?;
            }
            #[cfg(feature = "btrfs")]
            StorageMethod::Btrfs => {
                use std::io::ErrorKind;
                let dir = dir.clone();
                if let Some(parent) = dir.parent() {
                    fs::create_dir_all(parent).await?;
                }
                // TODO: Quota
                tokio::task::spawn_blocking(move || {
                    let _subvol = btrfsutil::subvolume::Subvolume::create(dir.as_path(), None)
                        .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                    Ok::<(), io::Error>(())
                })
                .await
                .map_err(|err| io::Error::new(ErrorKind::Other, err))??;
            }
        }
    }

    #[cfg(feature = "btrfs")]
    if let StorageMethod::Btrfs = config.storage.method {
        // TODO: Update quota
    }
    Ok(fs::canonicalize(dir).await?.to_string_lossy().to_string())
}

pub(crate) async fn delete(config: &DaemonConfig, id: StorageIdentifier) -> Result<(), io::Error> {
    let base = PathBuf::from(&config.storage.path);
    let dir = base.join(id.as_pathbuf());

    #[cfg(feature = "btrfs")]
    if let StorageMethod::Btrfs = config.storage.method {
        use btrfsutil::subvolume::DeleteFlags;
        use std::io::ErrorKind;
        return tokio::task::spawn_blocking(move || {
            let subvol = btrfsutil::subvolume::Subvolume::get(dir.as_path())
                .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
            subvol
                .delete(Some(DeleteFlags::RECURSIVE))
                .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
            Ok(())
        })
        .await
        .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
    }

    if dir.try_exists()? {
        fs::remove_dir_all(&dir).await?;
    }
    Ok(())
}
