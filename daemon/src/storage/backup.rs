use crate::config::{BackupMethod, DaemonConfig};
use crate::storage::StorageIdentifier;
use crate::utils::{ChannelStreamReceiver, ChannelStreamSender};
use std::io;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncRead;

#[allow(unused)]
pub(crate) async fn create_backup(
    config: &DaemonConfig,
    id: StorageIdentifier,
) -> Result<u64, io::Error> {
    let base = PathBuf::from(&config.storage.path);
    let dir = base.join(id.as_pathbuf());
    let target_dir = PathBuf::from(&config.backup.path).join(id.as_pathbuf());
    if !tokio::fs::try_exists(&target_dir).await? {
        tokio::fs::create_dir_all(&target_dir).await?;
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    match config.backup.method {
        BackupMethod::Archive { ref compression } => {
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(target_dir.join(format!("{}.tar{}", timestamp, compression.ext())))
                .await?;
            let encoder = compression.create_encoder(file);
            let mut archive = tokio_tar::Builder::new(encoder);
            archive.append_dir_all(".", &dir).await?;
            archive.finish().await?;
        }
        #[cfg(feature = "btrfs")]
        BackupMethod::Btrfs { .. } => {
            use btrfsutil::subvolume::SnapshotFlags;
            use btrfsutil::BtrfsUtilError;

            tokio::task::spawn_blocking(move || {
                let subvol = btrfsutil::subvolume::Subvolume::get(dir.as_path())?;
                let target = target_dir.join(timestamp.to_string());
                subvol.snapshot(target.as_path(), SnapshotFlags::READ_ONLY, None)?;
                Ok::<(), BtrfsUtilError>(())
            })
            .await
            .map_err(|err| io::Error::new(ErrorKind::Other, err))?
            .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
        }
    }
    Ok(timestamp)
}

#[allow(unused)]
pub(crate) async fn backup_to_stream(
    config: &DaemonConfig,
    id: StorageIdentifier,
    timestamp: u64,
) -> Result<Pin<Box<dyn AsyncRead + Send + Sync + 'static>>, io::Error> {
    let target_dir = PathBuf::from(&config.backup.path).join(id.as_pathbuf());
    match config.backup.method {
        BackupMethod::Archive { ref compression } => {
            let target = target_dir.join(format!("{}.tar{}", timestamp, compression.ext()));
            Ok(Box::pin(File::open(target).await?))
        }
        #[cfg(feature = "btrfs")]
        BackupMethod::Btrfs {
            ref jit_compression,
        } => {
            let compression = *jit_compression;
            let target = target_dir.join(timestamp.to_string());
            let (sender, recv) = tokio::sync::mpsc::channel(2);
            tokio::task::spawn(async move {
                let encoder = compression.create_encoder(ChannelStreamSender(Some(sender)));
                let mut archive = tokio_tar::Builder::new(encoder);
                archive.append_dir_all(".", &target).await?;
                archive.finish().await?;
                Ok::<(), io::Error>(())
            });
            Ok(Box::pin(ChannelStreamReceiver(recv)))
        }
    }
}

#[allow(unused)]
pub(crate) async fn delete_backup(
    config: &DaemonConfig,
    id: StorageIdentifier,
    timestamp: u64,
) -> Result<(), io::Error> {
    let target_dir = PathBuf::from(&config.backup.path).join(id.as_pathbuf());
    match config.backup.method {
        BackupMethod::Archive { ref compression } => {
            let target = target_dir.join(format!("{}.tar{}", timestamp, compression.ext()));
            tokio::fs::remove_file(target).await?;
        }
        #[cfg(feature = "btrfs")]
        BackupMethod::Btrfs { .. } => {
            use btrfsutil::subvolume::DeleteFlags;
            use std::io::ErrorKind;
            return tokio::task::spawn_blocking(move || {
                let target = target_dir.join(timestamp.to_string());
                let subvol = btrfsutil::subvolume::Subvolume::get(target.as_path())
                    .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                subvol
                    .delete(Some(DeleteFlags::RECURSIVE))
                    .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                Ok(())
            })
            .await
            .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
        }
    }
    Ok(())
}
