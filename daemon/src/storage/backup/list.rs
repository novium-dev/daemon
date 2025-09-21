// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::BackupEntry;
use crate::storage::StorageIdentifier;
use crate::storage::backup::BackupNameExt as _;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::{Daemon, DaemonExtra};
use daemon_common::service::backup::BackupResponseEntry;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use std::io;
use std::os::unix::fs::MetadataExt as _;
use std::path::PathBuf;
use tokio::fs::File;
use tracing::error;

/// Cancel safe
pub async fn list_backups<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
) -> Result<Vec<BackupResponseEntry>, E> {
    let StorageLockData { lock, .. } =
        storage_read_timeout::<E>(&daemon.config, &*daemon.conn, &daemon.storages, id).await?;
    let mut entries = Vec::new();
    for (storage, backup_storage) in &daemon.config.backup {
        let target_dir = PathBuf::from(backup_storage.path()).join(id.as_pathbuf());
        match backup_storage {
            BackupEntry::Archive { compression, .. } => {
                let mut readdir = tokio::fs::read_dir(&target_dir)
                    .await
                    .inspect_err(|err| error!("Failed to list backups (read_dir): {err:?}"))?;
                while let Some(entry) = readdir
                    .next_entry()
                    .await
                    .inspect_err(|err| error!("Failed to list backups (next_entry): {err:?}"))?
                {
                    let metadata = entry.metadata().await.inspect_err(|err| {
                        error!("Failed to list backups (metadata): {err:?}");
                    })?;

                    #[expect(
                        clippy::filetype_is_file,
                        reason = "Only list regular files here, not device files etc."
                    )]
                    if !metadata.file_type().is_file() {
                        continue;
                    }
                    let entry_path = entry.path();
                    let size = metadata.size();
                    let filename = entry.file_name().to_string_lossy().to_string();
                    let Some(entry) = filename.strip_suffix(&format!(".tar{}", compression.ext()))
                    else {
                        continue;
                    };
                    let entry: Result<i64, _> = entry.parse();

                    let file = File::open(entry_path)
                        .await
                        .inspect_err(|err| error!("Failed to list backups (open): {err:?}"))?;
                    let name = file.get_backup_name().await.inspect_err(|err| {
                        error!("Failed to list backups (get_name): {err:?}");
                    })?;
                    if let Ok(entry) = entry {
                        entries.push(BackupResponseEntry {
                            time: entry,
                            storage: storage.clone(),
                            size: Some(size),
                            name,
                        });
                    }
                }
            }
            #[cfg(feature = "btrfs")]
            BackupEntry::Btrfs { quota, .. } => {
                use std::collections::HashMap;
                let mut readdir = tokio::fs::read_dir(&target_dir).await?;

                let mut quotas = HashMap::new();
                if *quota {
                    quotas = tokio::task::spawn_blocking(move || {
                        if let Err(err) = super::super::btrfs::search_quotas(
                            std::fs::File::open(target_dir),
                            &mut quotas,
                        ) {
                            error!("Unable to retrieve quotas: {err:?}. Ensure quotas are enabled");
                        }
                        quotas
                    })
                    .await
                    .inspect_err(|err| {
                        error!("Failed to list backups (retrieve quotas): {err:?}");
                    })
                    .map_err(io::Error::from)?;
                }
                while let Some(entry) = readdir
                    .next_entry()
                    .await
                    .inspect_err(|err| error!("Failed to list backups (next_entry): {err:?}"))?
                {
                    let metadata = entry.metadata().await.inspect_err(|err| {
                        error!("Failed to list backups (metadata): {err:?}");
                    })?;
                    if !metadata.file_type().is_dir() {
                        continue;
                    }
                    let entry_path = entry.path();
                    let entry = entry.file_name().to_string_lossy().to_string();
                    let entry: Result<i64, _> = entry.parse();

                    let file = File::open(&entry_path)
                        .await
                        .inspect_err(|err| error!("Failed to list backups (open): {err:?}"))?;
                    let name = file.get_backup_name().await.inspect_err(|err| {
                        error!("Failed to list backups (get_name): {err:?}");
                    })?;
                    let size = if *quota {
                        match tokio::task::spawn_blocking(move || {
                            super::super::btrfs::get_subvol_id(Ok(file))
                        })
                        .await
                        .inspect_err(|err| {
                            error!("Failed to list backups (get_subvol_id): {err:?}");
                        })
                        .map_err(io::Error::from)?
                        {
                            Ok(subvol) => quotas.get(&subvol).copied(),
                            Err(err) => {
                                error!(
                                    "Unable to retrieve subvolume id: {err:?} in {:?}. Ensure this backup is a subvolume and not a directory!",
                                    entry_path.to_string_lossy(),
                                );
                                None
                            }
                        }
                    } else {
                        None
                    };

                    if let Ok(entry) = entry {
                        entries.push(BackupResponseEntry {
                            time: entry,
                            storage: storage.clone(),
                            size,
                            name,
                        });
                    }
                }
            }
        }
    }
    drop(lock);
    Ok(entries)
}
