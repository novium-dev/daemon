// SPDX-License-Identifier: AGPL-3.0-only
pub mod archive;
pub(crate) mod backup;
#[cfg(feature = "btrfs")]
pub mod btrfs;
pub(crate) mod directory;
#[cfg(target_os = "linux")]
pub mod ext4;
pub(crate) mod scan;
pub(crate) mod sync;
pub(crate) mod worker;

use crate::config::StorageEntry;
use crate::storage::worker::{ApplyQuotaToken, ResolvedStorage};
use async_trait::async_trait;
use core::sync::atomic::{AtomicI64, Ordering};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::error;

pub const STORAGE_TOKEN_MAGIC: &str = "*DAEMON_WATCH\0";
#[derive(Debug, Hash, PartialEq, Eq, Clone, Copy)]
pub enum StorageIdentifier {
    Primary(u32),
}

impl StorageIdentifier {
    #[must_use]
    pub fn as_pathbuf(self) -> PathBuf {
        match self {
            Self::Primary(id) => PathBuf::from(format!("primary/{id}")),
        }
    }

    /// Quota project id of this storage.
    #[cfg(target_os = "linux")]
    #[must_use]
    pub const fn project_id(self) -> u32 {
        match self {
            Self::Primary(id) => id,
        }
    }
}

#[async_trait]
pub trait StorageBackend {
    // Must be cancel safe
    async fn create(
        &self,
        id: StorageIdentifier,
        base: &Path,
        target: &Path,
    ) -> Result<(), io::Error>;

    // Must be cancel safe
    async fn apply_quota(
        &self,
        id: StorageIdentifier,
        base: &Path,
        target: &Path,
        limit: Option<u64>,
    ) -> Result<bool, io::Error>;

    // Must be cancel safe
    async fn delete(
        &self,
        id: StorageIdentifier,
        base: &Path,
        target: &Path,
    ) -> Result<(), io::Error>;

    // If successful, `result.is_some()` must match [`StorageBackend::apply_quota`].
    fn quota_usage<'a>(
        &self,
        base: &Path,
        storages: &'a HashMap<StorageIdentifier, ResolvedStorage>,
    ) -> Result<Option<ApplyQuotaToken<'a>>, io::Error>;
}

/// Cancel safe (possible subvolume without quota, should correct itself after next storage access)
pub async fn prepare(
    id: StorageIdentifier,
    storage: &StorageEntry,
    value: &(AtomicI64, AtomicI64),
    disk: Option<u64>,
) -> Result<String, io::Error> {
    let base = Path::new(&storage.path);
    let target = base.join(id.as_pathbuf());

    if !target
        .try_exists()
        .inspect_err(|err| error!("Unable to check existence of storage directory: {err:?}"))?
    {
        storage.backend.create(id, base, &target).await?;
    }

    storage.backend.apply_quota(id, base, &target, disk).await?;

    value.1.store(
        disk.filter(|value| *value <= i64::MAX.unsigned_abs())
            .map_or(-1, u64::cast_signed),
        Ordering::Relaxed,
    );

    Ok(fs::canonicalize(target)
        .await
        .inspect_err(|err| error!("Unable to canonicalize storage directory: {err:?}"))?
        .to_string_lossy()
        .to_string())
}

/// Cancel safe
pub async fn delete(storage: &StorageEntry, id: StorageIdentifier) -> Result<(), io::Error> {
    let base = Path::new(&storage.path);
    let target = base.join(id.as_pathbuf());

    storage.backend.delete(id, base, &target).await
}
