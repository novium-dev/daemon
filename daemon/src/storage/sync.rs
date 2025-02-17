use crate::config::{DaemonConfig, StorageEntry};
use crate::storage::{ensure_exists, StorageIdentifier};
use crate::sync::Mutex;
use crate::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use crate::utils::generic_server_select;
use crate::web::sql_error_to_http;
use crate::StorageData;
use axum::http::StatusCode;
use sqlx::{Executor, MySql};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::io;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::error::Elapsed;
use tracing::{debug, warn};

pub(crate) struct StorageLockData<'a, T> {
    pub(crate) path: String,
    pub(crate) lock: T,
    pub(crate) backup_exclude: HashSet<String>,
    pub(crate) storage: &'a StorageEntry,
    pub(crate) value: Arc<(AtomicI64, AtomicI64)>,
    pub(crate) stats_lock: Arc<Mutex<()>>,
    pub(crate) disk: Option<u64>,
}

async fn storage_lock<'a>(
    config: &'a DaemonConfig,
    conn: impl Executor<'_, Database = MySql> + Copy,
    storages: &Mutex<HashMap<StorageIdentifier, StorageData>>,
    storage: StorageIdentifier,
) -> Result<
    Option<(
        Option<u64>,
        Arc<RwLock<()>>,
        HashSet<String>,
        &'a StorageEntry,
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
    Ok(Some((disk, lock, backup_exclude, entry, value, stats_lock)))
}

pub(crate) async fn storage_read<'a>(
    config: &'a DaemonConfig,
    conn: impl Executor<'_, Database = MySql> + Copy,
    storages: &Mutex<HashMap<StorageIdentifier, StorageData>>,
    storage: StorageIdentifier,
    timeout: bool,
) -> Result<StorageLockData<'a, OwnedRwLockReadGuard<()>>, StorageError> {
    let Some((disk, lock, backup_exclude, entry, value, stats_lock)) =
        storage_lock(config, conn, storages, storage)
            .await
            .map_err(StorageError::Sqlx)?
    else {
        return Err(StorageError::NotFound);
    };
    let guard = if timeout {
        tokio::time::timeout(Duration::from_millis(100), lock.read_owned())
            .await
            .map_err(StorageError::Timeout)?
    } else {
        lock.read_owned().await
    };
    let path = ensure_exists(storage, entry, value.clone(), disk)
        .await
        .map_err(StorageError::IO)?;
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

pub(crate) async fn storage_write<'a>(
    config: &'a DaemonConfig,
    conn: impl Executor<'_, Database = MySql> + Copy,
    storages: &Mutex<HashMap<StorageIdentifier, StorageData>>,
    storage: StorageIdentifier,
    timeout: bool,
) -> Result<StorageLockData<'a, OwnedRwLockWriteGuard<()>>, StorageError> {
    let Some((disk, lock, backup_exclude, entry, value, stats_lock)) =
        storage_lock(config, conn, storages, storage)
            .await
            .map_err(StorageError::Sqlx)?
    else {
        return Err(StorageError::NotFound);
    };
    let guard = if timeout {
        tokio::time::timeout(Duration::from_millis(100), lock.write_owned())
            .await
            .map_err(StorageError::Timeout)?
    } else {
        lock.write_owned().await
    };
    let path = ensure_exists(storage, entry, value.clone(), disk)
        .await
        .map_err(StorageError::IO)?;
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
    IO(io::Error),
    Timeout(Elapsed),
    NotFound,
}

impl Error for StorageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            StorageError::Sqlx(err) => Some(err),
            StorageError::IO(err) => Some(err),
            StorageError::Timeout(err) => Some(err),
            StorageError::NotFound => None,
        }
    }
}

impl Display for StorageError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        <Self as Debug>::fmt(self, f)
    }
}

impl StorageError {
    pub(crate) fn into_http(self) -> StatusCode {
        match self {
            StorageError::Sqlx(err) => sql_error_to_http(err),
            StorageError::IO(err) => {
                warn!("Storage error: {:?}", err);
                StatusCode::INTERNAL_SERVER_ERROR
            }
            StorageError::Timeout(_) => {
                debug!("Storage lock timeout");
                StatusCode::SERVICE_UNAVAILABLE
            }
            StorageError::NotFound => StatusCode::NOT_FOUND,
        }
    }
}
