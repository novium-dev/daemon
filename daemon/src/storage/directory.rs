// SPDX-License-Identifier: AGPL-3.0-only

use std::io;
use std::path::Path;

use async_trait::async_trait;
use tracing::error;

use crate::storage::{
    StorageBackend, StorageIdentifier,
    worker::{ApplyQuotaToken, ResolvedStorage},
};
use std::collections::HashMap;

pub struct DirectoryStorageBackend;

#[async_trait]
impl StorageBackend for DirectoryStorageBackend {
    async fn create(
        &self,
        _id: StorageIdentifier,
        _base: &Path,
        target: &Path,
    ) -> Result<(), io::Error> {
        tokio::fs::create_dir_all(target)
            .await
            .inspect_err(|err| error!("Unable to create storage directory: {err:?}"))
    }

    async fn apply_quota(
        &self,
        _id: StorageIdentifier,
        _base: &Path,
        _target: &Path,
        _limit: Option<u64>,
    ) -> Result<bool, io::Error> {
        Ok(false)
    }

    async fn delete(
        &self,
        _id: StorageIdentifier,
        _base: &Path,
        target: &Path,
    ) -> Result<(), io::Error> {
        let target = target.to_owned();
        if tokio::fs::try_exists(&target)
            .await
            .inspect_err(|err| error!("Unable to check if storage directory exists: {err:?}"))?
        {
            tokio::fs::remove_dir_all(target)
                .await
                .inspect_err(|err| error!("Unable to remove storage directory: {err:?}"))?;
        }
        Ok(())
    }

    fn quota_usage<'a>(
        &self,
        _base: &Path,
        _storages: &'a HashMap<StorageIdentifier, ResolvedStorage>,
    ) -> Result<Option<ApplyQuotaToken<'a>>, io::Error> {
        Ok(None)
    }
}
