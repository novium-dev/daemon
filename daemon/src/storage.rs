use crate::config::DaemonConfig;
use std::io;
use std::io::ErrorKind;
use std::path::PathBuf;
use tokio::fs;

#[derive(Hash, PartialEq, Clone)]
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
        if config.btrfs {
            if let Some(parent) = dir.parent() {
                fs::create_dir_all(parent).await?;
            }
            // TODO: Quota
            btrfsutil::subvolume::Subvolume::create(dir.as_path(), None)
                .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
        } else {
            fs::create_dir_all(&dir).await?;
        }
    }

    if config.btrfs {
        // TODO: Update quota
    }
    Ok(fs::canonicalize(dir).await?.to_string_lossy().to_string())
}

pub(crate) async fn delete(config: &DaemonConfig, id: StorageIdentifiers) -> Result<(), io::Error> {
    let base = PathBuf::from(&config.storage_path);
    let dir = base.join(id.as_pathbuf());
    if dir.try_exists()? {
        fs::remove_dir_all(&dir).await?;
    }
    Ok(())
}
