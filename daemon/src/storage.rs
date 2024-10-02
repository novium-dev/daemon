use crate::config::DaemonConfig;
use btrfsutil::subvolume::DeleteFlags;
use std::io;
use std::io::ErrorKind;
use std::path::PathBuf;
use tokio::fs;

#[derive(Hash, PartialEq, Clone, Copy)]
pub(crate) enum StorageIdentifiers {
    Primary(u32),
}

impl StorageIdentifiers {
    pub(crate) fn as_pathbuf(&self) -> PathBuf {
        match self {
            StorageIdentifiers::Primary(id) => PathBuf::from(format!("primary/{}", id)),
        }
    }

    #[allow(unused)]
    pub(crate) fn as_vec(&self) -> Vec<String> {
        match self {
            StorageIdentifiers::Primary(id) => vec!["primary".to_owned(), id.to_string()],
        }
    }
}

pub(crate) async fn ensure_exists(
    config: &DaemonConfig,
    id: StorageIdentifiers,
    _disk: Option<u64>,
) -> Result<String, io::Error> {
    let base = PathBuf::from(&config.storage_path);
    let dir = base.join(id.as_pathbuf());

    if !dir.try_exists()? {
        #[cfg(feature = "btrfs")]
        if config.btrfs {
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
        } else {
            fs::create_dir_all(&dir).await?;
        }
        #[cfg(not(feature = "btrfs"))]
        fs::create_dir_all(&dir).await?;
    }

    #[cfg(feature = "btrfs")]
    if config.btrfs {
        // TODO: Update quota
    }
    Ok(fs::canonicalize(dir).await?.to_string_lossy().to_string())
}

pub(crate) async fn delete(config: &DaemonConfig, id: StorageIdentifiers) -> Result<(), io::Error> {
    let base = PathBuf::from(&config.storage_path);
    let dir = base.join(id.as_pathbuf());

    #[cfg(feature = "btrfs")]
    if config.btrfs {
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
