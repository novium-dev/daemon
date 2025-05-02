use crate::StorageData;
use crate::config::{DaemonConfig, StorageEntry};
use crate::storage::{StorageIdentifier, ensure_exists};
use crate::sync::Mutex;
use crate::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use crate::utils::generic_server_select;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use sqlx::{Executor, MySql};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::io;
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::time::Duration;

pub(crate) struct StorageLockData<T> {
    pub(crate) path: String,
    pub(crate) lock: T,
    pub(crate) backup_exclude: HashSet<String>,
    pub(crate) storage: StorageEntry,
    pub(crate) value: Arc<(AtomicI64, AtomicI64)>,
    pub(crate) stats_lock: Arc<Mutex<()>>,
    pub(crate) disk: Option<u64>,
}

async fn storage_lock(
    config: &DaemonConfig,
    conn: impl Executor<'_, Database = MySql> + Copy,
    storages: &Mutex<HashMap<StorageIdentifier, StorageData>>,
    storage: StorageIdentifier,
) -> Result<
    Option<(
        Option<u64>,
        Arc<RwLock<()>>,
        HashSet<String>,
        StorageEntry,
        Arc<(AtomicI64, AtomicI64)>,
        Arc<Mutex<()>>,
    )>,
    sqlx::Error,
> {
    let (disk, backup_exclude, entry) = match storage {
        StorageIdentifier::Primary(server_id) => {
            let Some(server) = generic_server_select(conn, server_id).await? else {
                return Ok(None);
            };
            let Some(storage) = config.storage.get(&server.storage) else {
                return Ok(None);
            };
            (server.disk, server.backup_exclude, storage)
        }
    };
    let mut storages = storages.lock().await;
    let (value, lock, stats_lock) = match storages.get(&storage) {
        None => {
            let lock = Arc::new(RwLock::new(()));
            let current = AtomicI64::new(-1);
            let max = AtomicI64::new(disk.map(|max| max as i64).unwrap_or(-1));
            let value = Arc::new((current, max));
            let stats_lock = Arc::new(Mutex::new(()));
            storages.insert(
                storage,
                StorageData {
                    lock: lock.clone(),
                    value: value.clone(),
                    stats_lock: stats_lock.clone(),
                },
            );
            (value, lock, stats_lock)
        }
        Some(lock) => (
            lock.value.clone(),
            lock.lock.clone(),
            lock.stats_lock.clone(),
        ),
    };
    Ok(Some((
        disk,
        lock,
        backup_exclude,
        entry.clone(),
        value,
        stats_lock,
    )))
}

pub(crate) async fn storage_read_timeout<
    'a,
    E: From<sqlx::Error> + FromStorageNotFound + FromStorageMutexLocked + FromStorageFailure,
>(
    config: &DaemonConfig,
    conn: impl Executor<'_, Database = MySql> + Copy,
    storages: &Mutex<HashMap<StorageIdentifier, StorageData>>,
    storage: StorageIdentifier,
) -> Result<StorageLockData<OwnedRwLockReadGuard<()>>, E> {
    let Some((disk, lock, backup_exclude, entry, value, stats_lock)) =
        storage_lock(config, conn, storages, storage).await?
    else {
        return Err(<E as FromStorageNotFound>::from());
    };
    let guard = tokio::time::timeout(Duration::from_millis(100), lock.read_owned())
        .await
        .map_err(|_| <E as FromStorageMutexLocked>::from())?;
    let path = ensure_exists(storage, &entry, value.clone(), disk).await
        .map_err(|_| <E as FromStorageFailure>::from())?;
    Ok(StorageLockData {
        path,
        lock: guard,
        backup_exclude,
        storage: entry,
        value,
        stats_lock,
        disk,
    })
}

pub(crate) async fn storage_read<
    'a,
    E: From<sqlx::Error> + FromStorageNotFound + FromStorageFailure,
>(
    config: &DaemonConfig,
    conn: impl Executor<'_, Database = MySql> + Copy,
    storages: &Mutex<HashMap<StorageIdentifier, StorageData>>,
    storage: StorageIdentifier,
) -> Result<StorageLockData<OwnedRwLockReadGuard<()>>, E> {
    let Some((disk, lock, backup_exclude, entry, value, stats_lock)) =
        storage_lock(config, conn, storages, storage).await?
    else {
        return Err(<E as FromStorageNotFound>::from());
    };
    let guard = lock.read_owned().await;
    let path = ensure_exists(storage, &entry, value.clone(), disk).await
        .map_err(|_| <E as FromStorageFailure>::from())?;
    Ok(StorageLockData {
        path,
        lock: guard,
        backup_exclude,
        storage: entry,
        value,
        stats_lock,
        disk,
    })
}

#[allow(dead_code)]
pub(crate) async fn storage_write<
    'a,
    E: From<sqlx::Error> + FromStorageNotFound + FromStorageFailure,
>(
    config: &DaemonConfig,
    conn: impl Executor<'_, Database = MySql> + Copy,
    storages: &Mutex<HashMap<StorageIdentifier, StorageData>>,
    storage: StorageIdentifier,
) -> Result<StorageLockData<OwnedRwLockWriteGuard<()>>, E> {
    let Some((disk, lock, backup_exclude, entry, value, stats_lock)) =
        storage_lock(config, conn, storages, storage).await?
    else {
        return Err(<E as FromStorageNotFound>::from());
    };
    let guard = lock.write_owned().await;
    let path = ensure_exists(storage, &entry, value.clone(), disk).await
        .map_err(|_| <E as FromStorageFailure>::from())?;
    Ok(StorageLockData {
        path,
        lock: guard,
        backup_exclude,
        storage: entry,
        value,
        stats_lock,
        disk,
    })
}

pub(crate) async fn storage_write_timeout<
    'a,
    E: From<sqlx::Error> + FromStorageNotFound + FromStorageMutexLocked + FromStorageFailure,
>(
    config: &DaemonConfig,
    conn: impl Executor<'_, Database = MySql> + Copy,
    storages: &Mutex<HashMap<StorageIdentifier, StorageData>>,
    storage: StorageIdentifier,
) -> Result<StorageLockData<OwnedRwLockWriteGuard<()>>, E> {
    let Some((disk, lock, backup_exclude, entry, value, stats_lock)) =
        storage_lock(config, conn, storages, storage).await?
    else {
        return Err(<E as FromStorageNotFound>::from());
    };
    let guard = tokio::time::timeout(Duration::from_millis(100), lock.write_owned())
        .await
        .map_err(|_| <E as FromStorageMutexLocked>::from())?;
    let path = ensure_exists(storage, &entry, value.clone(), disk).await
        .map_err(|_| <E as FromStorageFailure>::from())?;
    Ok(StorageLockData {
        path,
        lock: guard,
        backup_exclude,
        storage: entry,
        value,
        stats_lock,
        disk,
    })
}

#[derive(Debug)]
pub(crate) enum StorageError {
    Sqlx(sqlx::Error),
    Failure,
    IO(io::Error),
    Timeout,
    NotFound,
}

impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            StorageError::Sqlx(err) => Some(err),
            StorageError::IO(err) => Some(err),
            StorageError::Timeout => None,
            StorageError::NotFound => None,
            StorageError::Failure => None,
        }
    }
}

impl From<io::Error> for StorageError {
    fn from(value: io::Error) -> Self {
        Self::IO(value)
    }
}

impl From<sqlx::Error> for StorageError {
    fn from(value: sqlx::Error) -> Self {
        Self::Sqlx(value)
    }
}

impl FromStorageNotFound for StorageError {
    fn from() -> Self
    where
        Self: Sized,
    {
        Self::NotFound
    }
}

impl FromStorageMutexLocked for StorageError {
    fn from() -> Self
    where
        Self: Sized,
    {
        Self::Timeout
    }
}

impl FromStorageFailure for StorageError {
    fn from() -> Self
    where
        Self: Sized
    {
        Self::Failure
    }
}

impl Display for StorageError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        <Self as Debug>::fmt(self, f)
    }
}

pub(crate) struct MapStorageNotFoundIsFailure<T: FromStorageFailure>(T);

impl<T: FromStorageFailure> FromStorageNotFound for MapStorageNotFoundIsFailure<T> {
    fn from() -> Self
    where
        Self: Sized,
    {
        MapStorageNotFoundIsFailure(<T as FromStorageFailure>::from())
    }
}

impl<T: From<io::Error> + FromStorageFailure> From<io::Error> for MapStorageNotFoundIsFailure<T> {
    fn from(value: io::Error) -> Self {
        Self(<T as From<io::Error>>::from(value))
    }
}

impl<T: From<sqlx::Error> + FromStorageFailure> From<sqlx::Error>
    for MapStorageNotFoundIsFailure<T>
{
    fn from(value: sqlx::Error) -> Self {
        Self(<T as From<sqlx::Error>>::from(value))
    }
}

impl<T: FromStorageMutexLocked + FromStorageFailure> FromStorageMutexLocked
    for MapStorageNotFoundIsFailure<T>
{
    fn from() -> Self
    where
        Self: Sized,
    {
        Self(<T as FromStorageMutexLocked>::from())
    }
}
