// SPDX-License-Identifier: AGPL-3.0-only
use crate::files::storage_path;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageLockData, StorageTokens, storage_read_timeout};
use crate::utils::storage_parent_ino;
use crate::{Daemon, DaemonExtra, StorageNotification};
use cap_async_std::fs::{Dir, MetadataExt as _, OpenOptions};
use core::fmt::Write as _;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::task::{FileOperationKind, StorageTask};
use daemon_common::service::files::{
    FileOperation, FileOperationBatch, FileOperationFailure, FileOperationResult, UrlWrite,
    UrlWriteBatch, UrlWriteFailure, UrlWriteResult,
};
use futures::StreamExt as _;
use sha2::{Digest as _, Sha512};
use std::io;
use std::io::ErrorKind;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncWriteExt as _, BufWriter};
use tokio::sync::broadcast::Sender;
use tracing::{debug, warn};

const DEFAULT_PARALLEL: u8 = 8;
const MAX_PARALLEL: u8 = 16;
const DEFAULT_ATTEMPTS: u8 = 3;
const MAX_ATTEMPTS: u8 = 5;
const PART_SUFFIX: &str = ".part";

fn url_failure(err: &io::Error) -> UrlWriteFailure {
    match err.kind() {
        ErrorKind::NotFound => UrlWriteFailure::NotFound,
        ErrorKind::PermissionDenied => UrlWriteFailure::SandboxViolated,
        ErrorKind::IsADirectory => UrlWriteFailure::IsDirectory,
        ErrorKind::QuotaExceeded => UrlWriteFailure::QuotaReached,
        _ => UrlWriteFailure::Io,
    }
}

fn operation_failure(err: &io::Error) -> FileOperationFailure {
    match err.kind() {
        ErrorKind::NotFound => FileOperationFailure::NotFound,
        ErrorKind::AlreadyExists | ErrorKind::DirectoryNotEmpty => {
            FileOperationFailure::AlreadyExists
        }
        ErrorKind::PermissionDenied => FileOperationFailure::SandboxViolated,
        ErrorKind::IsADirectory => FileOperationFailure::IsDirectory,
        ErrorKind::NotADirectory => FileOperationFailure::NotDirectory,
        ErrorKind::QuotaExceeded => FileOperationFailure::QuotaReached,
        _ => FileOperationFailure::Io,
    }
}

const fn retryable(failure: UrlWriteFailure) -> bool {
    matches!(
        failure,
        UrlWriteFailure::RequestFailed
            | UrlWriteFailure::NotSuccess
            | UrlWriteFailure::ReadFailed
            | UrlWriteFailure::SizeMismatch
            | UrlWriteFailure::HashMismatch
            | UrlWriteFailure::Io
    )
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn part_path(path: &Path) -> PathBuf {
    let mut part = path.as_os_str().to_owned();
    part.push(PART_SUFFIX);
    PathBuf::from(part)
}

async fn create_parents(storage: &Dir, path: &Path) -> Result<(), io::Error> {
    let Some(parent) = path.parent().map(Path::to_path_buf) else {
        return Ok(());
    };
    let storage = Dir::clone(storage);
    tokio::task::spawn_blocking(move || storage.create_dir_all(&parent))
        .await
        .map_err(io::Error::from)?
}

fn notify_update(notify: &Sender<StorageNotification>, parent: Option<u64>, inode: Option<u64>) {
    let _ = notify
        .send(StorageNotification::Update {
            parent_inode: parent,
            inode,
        })
        .inspect_err(|err| warn!("Unable to trigger storage notification: {err:?}"));
}

async fn download_once(
    daemon: &Daemon<impl DaemonExtra>,
    storage: &Dir,
    part: &Path,
    file: &UrlWrite,
) -> Result<(), UrlWriteFailure> {
    let mut response = daemon
        .client
        .get(file.url.clone())
        .send()
        .await
        .inspect_err(|err| warn!("Batch download of {} failed (request): {err:?}", file.url))
        .map_err(|_err| UrlWriteFailure::RequestFailed)?
        .error_for_status()
        .inspect_err(|err| warn!("Batch download of {} failed (status): {err:?}", file.url))
        .map_err(|_err| UrlWriteFailure::NotSuccess)?;

    let fd: OwnedFd = storage
        .open_with(
            part,
            OpenOptions::new().create(true).truncate(true).write(true),
        )
        .await
        .inspect_err(|err| debug!("Batch download could not open {part:?}: {err:?}"))
        .map_err(|err| url_failure(&err))?
        .into_std()
        .into();
    let mut writer = BufWriter::new(tokio::fs::File::from_std(std::fs::File::from(fd)));

    let size_limit = file.size.filter(|_size| file.sha512.is_none());
    let mut hasher = Sha512::new();
    let mut written: u64 = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .inspect_err(|err| warn!("Batch download of {} failed (read): {err:?}", file.url))
        .map_err(|_err| UrlWriteFailure::ReadFailed)?
    {
        written = written.saturating_add(chunk.len() as u64);
        if size_limit.is_some_and(|size| written > size) {
            return Err(UrlWriteFailure::SizeMismatch);
        }
        hasher.update(&chunk);
        writer
            .write_all(&chunk)
            .await
            .map_err(|err| url_failure(&err))?;
    }
    writer.flush().await.map_err(|err| url_failure(&err))?;
    drop(writer);

    if let Some(expected) = &file.sha512 {
        if !hex(&hasher.finalize()).eq_ignore_ascii_case(expected) {
            return Err(UrlWriteFailure::HashMismatch);
        }
        if file.size.is_some_and(|size| written != size) {
            debug!(
                "{} has {written} bytes instead of the declared size, but its hash matches",
                file.url
            );
        }
    } else if size_limit.is_some_and(|size| written != size) {
        return Err(UrlWriteFailure::SizeMismatch);
    } else {
        debug!(
            "{} has no declared hash, only its size was checked",
            file.url
        );
    }

    Ok(())
}

async fn write_one(
    daemon: &Daemon<impl DaemonExtra>,
    storage: Dir,
    path: PathBuf,
    file: &UrlWrite,
    attempts: u8,
    notify: &Sender<StorageNotification>,
) -> Result<(), UrlWriteFailure> {
    create_parents(&storage, &path)
        .await
        .map_err(|err| url_failure(&err))?;
    let part = part_path(&path);

    let mut last = UrlWriteFailure::Io;
    for attempt in 1..=attempts {
        match download_once(daemon, &storage, &part, file).await {
            Ok(()) => {
                storage
                    .rename(&part, &storage, &path)
                    .await
                    .map_err(|err| url_failure(&err))?;
                let parent = storage_parent_ino(&storage, &path).await.ok();
                let inode = storage.metadata(&path).await.map(|meta| meta.ino()).ok();
                notify_update(notify, parent, inode);
                return Ok(());
            }
            Err(failure) => {
                let _ = storage.remove_file(&part).await;
                debug!("Batch download attempt {attempt} of {path:?} failed: {failure:?}");
                if !retryable(failure) {
                    return Err(failure);
                }
                last = failure;
            }
        }
    }

    Err(last)
}

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
    let attempts = batch
        .attempts
        .unwrap_or(DEFAULT_ATTEMPTS)
        .clamp(1, MAX_ATTEMPTS);

    let (entry, token_map, sender) = (&storage, &tokens, &notify);
    let results = futures::stream::iter(batch.files)
        .map(|file| async move {
            let error = match storage_path(id, entry, file.path.clone(), token_map).await {
                Ok((dir, _, path)) => write_one(daemon, dir, path, &file, attempts, sender)
                    .await
                    .err(),
                Err(err) => Some(url_failure(&err)),
            };
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

async fn resolve(
    id: StorageIdentifier,
    storage: &crate::config::StorageEntry,
    tokens: &StorageTokens,
    path: String,
) -> Result<(Dir, PathBuf), io::Error> {
    let (dir, _, path) = storage_path(id, storage, path, tokens).await?;
    Ok((dir, path))
}

async fn exists(storage: &Dir, path: &Path) -> bool {
    storage.symlink_metadata(path).await.is_ok()
}

async fn delete_path(storage: &Dir, path: PathBuf, recursive: bool) -> Result<(), io::Error> {
    let metadata = match storage.symlink_metadata(&path).await {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };

    if metadata.is_dir() {
        if !recursive {
            return Err(io::Error::from(ErrorKind::IsADirectory));
        }
        let storage = Dir::clone(storage);
        tokio::task::spawn(async move { storage.remove_dir_all(&path).await })
            .await
            .map_err(io::Error::from)?
    } else {
        storage.remove_file(&path).await
    }
}

async fn move_path(
    from_dir: &Dir,
    from: &Path,
    to_dir: &Dir,
    to: &Path,
    overwrite: bool,
) -> Result<(), io::Error> {
    if !exists(from_dir, from).await {
        return Err(io::Error::from(ErrorKind::NotFound));
    }
    if exists(to_dir, to).await {
        if !overwrite {
            return Err(io::Error::from(ErrorKind::AlreadyExists));
        }
        delete_path(to_dir, to.to_path_buf(), true).await?;
    }
    create_parents(to_dir, to).await?;
    from_dir.rename(from, to_dir, to).await
}

async fn copy_path(
    from_dir: &Dir,
    from: &Path,
    to_dir: &Dir,
    to: &Path,
    overwrite: bool,
) -> Result<(), io::Error> {
    let mut queue = vec![(from.to_path_buf(), to.to_path_buf())];

    while let Some((source, target)) = queue.pop() {
        let metadata = from_dir.symlink_metadata(&source).await?;
        if metadata.is_symlink() {
            continue;
        }

        if metadata.is_dir() {
            create_parents(to_dir, &target.join("_")).await?;
            for entry in from_dir.read_dir(&source).await? {
                let name = entry?.file_name();
                queue.push((source.join(&name), target.join(&name)));
            }
            continue;
        }

        if exists(to_dir, &target).await {
            if !overwrite {
                return Err(io::Error::from(ErrorKind::AlreadyExists));
            }
            delete_path(to_dir, target.clone(), false).await?;
        }
        create_parents(to_dir, &target).await?;
        from_dir.copy(&source, to_dir, &target).await?;
    }

    Ok(())
}

async fn run_operation(
    id: StorageIdentifier,
    storage: &crate::config::StorageEntry,
    tokens: &StorageTokens,
    notify: &Sender<StorageNotification>,
    operation: FileOperation,
) -> Result<(), io::Error> {
    let touched = match operation {
        FileOperation::Delete { path, recursive } => {
            let (dir, path) = resolve(id, storage, tokens, path).await?;
            let parent = storage_parent_ino(&dir, &path).await.ok();
            delete_path(&dir, path, recursive).await?;
            parent
        }
        FileOperation::Mkdir { path } => {
            let (dir, path) = resolve(id, storage, tokens, path).await?;
            create_parents(&dir, &path.join("_")).await?;
            storage_parent_ino(&dir, &path).await.ok()
        }
        FileOperation::Move {
            from,
            to,
            overwrite,
        } => {
            let (from_dir, from) = resolve(id, storage, tokens, from).await?;
            let (to_dir, to) = resolve(id, storage, tokens, to).await?;
            if let Ok(parent) = storage_parent_ino(&from_dir, &from).await {
                notify_update(notify, Some(parent), None);
            }
            move_path(&from_dir, &from, &to_dir, &to, overwrite).await?;
            storage_parent_ino(&to_dir, &to).await.ok()
        }
        FileOperation::Copy {
            from,
            to,
            overwrite,
        } => {
            let (from_dir, from) = resolve(id, storage, tokens, from).await?;
            let (to_dir, to) = resolve(id, storage, tokens, to).await?;
            copy_path(&from_dir, &from, &to_dir, &to, overwrite).await?;
            storage_parent_ino(&to_dir, &to).await.ok()
        }
    };

    notify_update(notify, touched, None);
    Ok(())
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
            });
            continue;
        }

        let error = run_operation(id, &storage, &tokens, &notify, operation)
            .await
            .inspect_err(|err| debug!("Batch file operation failed: {err:?}"))
            .err()
            .map(|err| operation_failure(&err));
        failed |= error.is_some();
        results.push(FileOperationResult { error });
    }

    drop(lock);
    Ok(results)
}
