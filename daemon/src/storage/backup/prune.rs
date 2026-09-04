// SPDX-License-Identifier: AGPL-3.0-only
use crate::storage::StorageIdentifier;
use crate::storage::backup::{delete_backup, list_backups};
use crate::storage::sync::StorageError;
use crate::{Daemon, DaemonExtra};
use chrono_tz::Tz;
use core::cmp::Reverse;
use daemon_common::service::backup::error_backup_storage_location_not_found::FromBackupStorageLocationNotFound;
use daemon_common::service::backup::{BackupPruneEntry, BackupRetention, RetentionCandidate};
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use std::collections::HashMap;
use std::io::ErrorKind;
use tracing::{debug, info, warn};

/// Applies `retention` to the backups of `id` and deletes everything it does not
/// keep.
///
/// Each backup storage is pruned on its own, so a policy asking for seven daily
/// backups yields seven daily backups per storage rather than seven in total.
/// When `backup_storage` names a storage, only that one is considered; the
/// backups on every other storage are left alone and not reported.
///
/// A `dry_run` reports the same decisions without deleting anything.
pub async fn prune_backups<
    E: From<std::io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure
        + FromBackupStorageLocationNotFound,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    backup_storage: Option<&String>,
    retention: BackupRetention,
    timezone: Tz,
    dry_run: bool,
) -> Result<Vec<BackupPruneEntry>, E> {
    if let Some(backup_storage) = backup_storage
        && !daemon.config.backup.contains_key(backup_storage)
    {
        debug!("Backup storage does not exist: {backup_storage}");
        return Err(<E as FromBackupStorageLocationNotFound>::from());
    }

    let mut per_storage: HashMap<String, Vec<RetentionCandidate>> = HashMap::new();
    for entry in list_backups::<E>(daemon, id).await? {
        if backup_storage.is_some_and(|storage| storage != &entry.storage) {
            continue;
        }
        per_storage
            .entry(entry.storage)
            .or_default()
            .push(RetentionCandidate {
                id: entry.time,
                protected: entry.protected,
            });
    }

    let mut result = Vec::new();
    for (storage, candidates) in per_storage {
        let protected: HashMap<i64, bool> = candidates
            .iter()
            .map(|candidate| (candidate.id, candidate.protected))
            .collect();
        let outcome = retention.apply(&candidates, timezone);

        for backup_id in outcome.keep {
            result.push(BackupPruneEntry {
                time: backup_id,
                storage: storage.clone(),
                keep: true,
                protected: protected.get(&backup_id).copied().unwrap_or_default(),
            });
        }
        for backup_id in outcome.remove {
            if !dry_run {
                info!("Retention policy removes backup {backup_id} of {id:?} on {storage}");
                match delete_backup::<StorageError>(daemon, id, backup_id, &storage).await {
                    Ok(()) => {}
                    // Gone already: the policy wanted it gone, so this is done.
                    Err(StorageError::IO(err)) if err.kind() == ErrorKind::NotFound => {
                        warn!("Backup {backup_id} of {id:?} on {storage} was already gone");
                    }
                    Err(err) => return Err(err.into_service_error()),
                }
            }
            result.push(BackupPruneEntry {
                time: backup_id,
                storage: storage.clone(),
                keep: false,
                protected: protected.get(&backup_id).copied().unwrap_or_default(),
            });
        }
    }
    // Newest first, so the caller sees the same order as a backup listing.
    result.sort_unstable_by_key(|entry| Reverse(entry.time));
    Ok(result)
}
