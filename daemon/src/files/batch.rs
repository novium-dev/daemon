// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::StorageEntry;
use crate::files::copy::copy_file_locked;
use crate::files::delete::delete_file_locked;
use crate::files::list::list_files_locked;
use crate::files::mkdir::mkdir_file_locked;
use crate::files::rename::rename_file_locked;
use crate::files::stat::stat_file_locked;
use crate::files::write::write_file_by_url_locked;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageLockData, StorageTokens, storage_read_timeout};
use crate::{Daemon, DaemonExtra, StorageNotification};
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::task::{FileOperationKind, StorageTask};
use daemon_common::service::files::{
    BatchOperationError, FileOperation, FileOperationBatch, FileOperationFailure,
    FileOperationResult, UrlWriteBatch, UrlWriteResult,
};
use futures::StreamExt as _;
use std::io;
use tokio::sync::broadcast::Sender;

const DEFAULT_PARALLEL: u8 = 8;
const MAX_PARALLEL: u8 = 16;

/// Cancel safe
pub async fn write_files_by_url<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    batch: UrlWriteBatch,
) -> Result<Vec<UrlWriteResult>, E> {
    let StorageLockData {
        lock,
        storage,
        notify,
        tokens,
        ..
    } = storage_read_timeout::<E>(
        &daemon.config,
        &*daemon.conn,
        &daemon.storages,
        id,
        StorageTask::FileOperation {
            kind: FileOperationKind::WriteByUrlBatch,
        },
    )
    .await?;

    let parallel = usize::from(
        batch
            .parallel
            .unwrap_or(DEFAULT_PARALLEL)
            .clamp(1, MAX_PARALLEL),
    );
    let (entry, token_map, sender) = (&storage, &tokens, &notify);
    let results = futures::stream::iter(batch.files)
        .map(|file| async move {
            let error = write_file_by_url_locked::<BatchOperationError>(
                daemon,
                id,
                entry,
                token_map,
                sender,
                file.path.clone(),
                file.url,
            )
            .await
            .err()
            .map(FileOperationFailure::from);
            UrlWriteResult {
                path: file.path,
                error,
            }
        })
        .buffered(parallel)
        .collect::<Vec<_>>()
        .await;

    drop(lock);
    Ok(results)
}

async fn run_operation(
    id: StorageIdentifier,
    storage: &StorageEntry,
    tokens: &StorageTokens,
    notify: &Sender<StorageNotification>,
    operation: FileOperation,
) -> Result<FileOperationResult, BatchOperationError> {
    let mut result = FileOperationResult::default();
    match operation {
        FileOperation::List { path } => {
            result.entries =
                Some(list_files_locked::<BatchOperationError>(id, storage, tokens, path).await?);
        }
        FileOperation::Stat { path } => {
            result.entry =
                Some(stat_file_locked::<BatchOperationError>(id, storage, tokens, path).await?);
        }
        FileOperation::Delete { path, recursive } => {
            delete_file_locked::<BatchOperationError>(id, storage, tokens, notify, path, recursive)
                .await?;
        }
        FileOperation::Rename { from, to } => {
            rename_file_locked::<BatchOperationError>(id, storage, tokens, notify, from, to)
                .await?;
        }
        FileOperation::Copy { from, to } => {
            copy_file_locked::<BatchOperationError>(id, storage, tokens, notify, from, to).await?;
        }
        FileOperation::Mkdir { path, recursive } => {
            mkdir_file_locked::<BatchOperationError>(id, storage, tokens, notify, path, recursive)
                .await?;
        }
    }
    Ok(result)
}

/// Cancel safe
pub async fn file_operations<
    E: From<io::Error>
        + From<sqlx::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure,
>(
    daemon: &Daemon<impl DaemonExtra>,
    id: StorageIdentifier,
    batch: FileOperationBatch,
) -> Result<Vec<FileOperationResult>, E> {
    let StorageLockData {
        lock,
        storage,
        notify,
        tokens,
        ..
    } = storage_read_timeout::<E>(
        &daemon.config,
        &*daemon.conn,
        &daemon.storages,
        id,
        StorageTask::FileOperation {
            kind: FileOperationKind::Batch,
        },
    )
    .await?;

    let mut results = Vec::with_capacity(batch.operations.len());
    let mut failed = false;
    for operation in batch.operations {
        if failed && batch.stop_on_error {
            results.push(FileOperationResult {
                error: Some(FileOperationFailure::Skipped),
                ..FileOperationResult::default()
            });
            continue;
        }

        let result = run_operation(id, &storage, &tokens, &notify, operation)
            .await
            .unwrap_or_else(|err| FileOperationResult {
                error: Some(err.into()),
                ..FileOperationResult::default()
            });
        failed |= result.error.is_some();
        results.push(result);
    }

    drop(lock);
    Ok(results)
}
