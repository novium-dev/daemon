use async_trait::async_trait;
use axum::http::StatusCode;
use daemon_common::server::ServerStats;
use sqlx::{query_as, Executor, MySql, Pool};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::future::Future;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use std::{io, mem};
use tokio::sync::{Mutex, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};
use tokio::time::error::Elapsed;
use tokio_util::io::SyncIoBridge;
use tracing::{debug, warn};

use crate::config::{DaemonConfig, StorageEntry};
use crate::server::status::InternalServerState;
use crate::server::{FetchedServer, Server};
use crate::storage::{ensure_exists, StorageIdentifier};
use crate::web::sql_error_to_http;
use crate::ServerData;

pub(crate) trait ResourceData {
    fn into_u64(self) -> u64;
}

impl ResourceData for u32 {
    fn into_u64(self) -> u64 {
        self as u64
    }
}

impl ResourceData for u64 {
    fn into_u64(self) -> u64 {
        self
    }
}

pub(crate) fn from_mib(mib: impl ResourceData) -> u64 {
    mib.into_u64() * 1024 * 1024
}

#[cfg(feature = "database")]
pub(crate) async fn database(
    conn: impl Executor<'_, Database = MySql> + Copy,
    database_id: u32,
) -> Result<Option<crate::database::FetchedDatabase>, sqlx::Error> {
    let Some(database) = query_as!(
        crate::database::Database,
        "SELECT * FROM `databases` WHERE `id` = ? LIMIT 1",
        database_id
    )
    .fetch_optional(conn)
    .await?
    else {
        return Ok(None);
    };

    Ok(Some(database.fetched(conn).await?))
}

pub(crate) async fn server(
    conn: &Pool<MySql>,
    servers: &Mutex<HashMap<u32, Arc<Mutex<ServerData>>>>,
    server_id: u32,
) -> Result<Option<(FetchedServer, Arc<Mutex<ServerData>>)>, sqlx::Error> {
    let Some(server) = generic_server_select(conn, server_id).await? else {
        return Ok(None);
    };
    let mut servers = servers.lock().await;
    servers
        .entry(server_id)
        .or_insert_with(|| Arc::new(Mutex::new(ServerData::default())));
    Ok(Some((server, servers.get(&server_id).unwrap().clone())))
}

pub(crate) fn path_relative(str: String) -> PathBuf {
    if str.starts_with("/") {
        PathBuf::from(format!(".{}", str))
    } else {
        PathBuf::from(str)
    }
}

async fn storage_lock<'a>(
    config: &'a DaemonConfig,
    conn: impl Executor<'_, Database = MySql> + Copy,
    storages: &Mutex<HashMap<StorageIdentifier, Arc<RwLock<()>>>>,
    storage: StorageIdentifier,
) -> Result<
    Option<(
        Option<u64>,
        Arc<RwLock<()>>,
        HashSet<String>,
        &'a StorageEntry,
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
    let lock = match storages.get(&storage) {
        None => {
            let lock = Arc::new(RwLock::new(()));
            storages.insert(storage, lock.clone());
            lock
        }
        Some(lock) => lock.clone(),
    };
    Ok(Some((disk, lock, backup_exclude, entry)))
}

pub(crate) async fn storage_read<'a>(
    config: &'a DaemonConfig,
    conn: impl Executor<'_, Database = MySql> + Copy,
    storages: &Mutex<HashMap<StorageIdentifier, Arc<RwLock<()>>>>,
    storage: StorageIdentifier,
    timeout: bool,
) -> Result<
    (
        String,
        OwnedRwLockReadGuard<()>,
        HashSet<String>,
        &'a StorageEntry,
    ),
    StorageError,
> {
    let Some((disk, lock, backup_exclude, entry)) = storage_lock(config, conn, storages, storage)
        .await
        .map_err(StorageError::Sqlx)?
    else {
        return Err(StorageError::NotFound);
    };
    let (guard, backup_exclude, entry) = if timeout {
        (
            tokio::time::timeout(Duration::from_millis(100), lock.read_owned())
                .await
                .map_err(StorageError::Timeout)?,
            backup_exclude,
            entry,
        )
    } else {
        (lock.read_owned().await, backup_exclude, entry)
    };
    let path = ensure_exists(storage, entry, disk)
        .await
        .map_err(StorageError::IO)?;
    Ok((path, guard, backup_exclude, entry))
}

#[allow(unused)]
pub(crate) async fn storage_write<'a>(
    config: &'a DaemonConfig,
    conn: impl Executor<'_, Database = MySql> + Copy,
    storages: &Mutex<HashMap<StorageIdentifier, Arc<RwLock<()>>>>,
    storage: StorageIdentifier,
    timeout: bool,
) -> Result<
    (
        String,
        OwnedRwLockWriteGuard<()>,
        Option<u64>,
        &'a StorageEntry,
    ),
    StorageError,
> {
    let Some((disk, lock, _, entry)) = storage_lock(config, conn, storages, storage)
        .await
        .map_err(StorageError::Sqlx)?
    else {
        return Err(StorageError::NotFound);
    };
    let (guard, backup_exclude, entry) = if timeout {
        (
            tokio::time::timeout(Duration::from_millis(100), lock.write_owned())
                .await
                .map_err(StorageError::Timeout)?,
            disk,
            entry,
        )
    } else {
        (lock.write_owned().await, disk, entry)
    };
    let path = ensure_exists(storage, entry, disk)
        .await
        .map_err(StorageError::IO)?;
    Ok((path, guard, backup_exclude, entry))
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

pub(crate) async fn generic_server_select(
    conn: impl Executor<'_, Database = MySql> + Copy,
    server_id: u32,
) -> Result<Option<FetchedServer>, sqlx::Error> {
    let server: Option<Server> =
        query_as!(Server, "SELECT `id`,`image`,`cpu`,`ram`,`swap`,`disk`,`storage`,`internet` as `internet: bool`,`ipv6_suffix`,`uid`,`gid` FROM `servers` WHERE `id` = ? LIMIT 1", server_id)
            .fetch_optional(conn)
            .await?;
    match server {
        Some(server) => server.fetched(conn).await.map(Some),
        None => Ok(None),
    }
}

pub(crate) fn parse_path(path: &mut String) -> Result<(Vec<&str>, bool), ()> {
    let ends_with_slash = path.ends_with('/');
    if ends_with_slash {
        path.pop();
    }
    if path.starts_with('.') {
        path.remove(0);
    }
    if path.starts_with('/') {
        path.remove(0);
    }
    let components = if path.is_empty() {
        Vec::new()
    } else {
        path.split('/').collect()
    };
    short_path(components).map(|vec| (vec, ends_with_slash))
}

pub(crate) fn short_path(components: Vec<&str>) -> Result<Vec<&str>, ()> {
    let mut result = Vec::new();
    for component in components {
        if component.eq("..") {
            if !result.is_empty() {
                result.remove(result.len() - 1);
            } else {
                return Err(());
            }
        } else if !component.eq(".") {
            result.push(component);
        }
    }
    Ok(result)
}

#[allow(unused)]
#[derive(Clone)]
pub(crate) enum ChannelWriteData {
    Terminal(Vec<u8>),
    Status(InternalServerState),
    Stats(ServerStats),
    Ping,
}

#[allow(unused)]
#[async_trait]
pub(crate) trait ChannelAdapter<E: Send + 'static, J: Send + 'static, Q: Send + 'static> {
    async fn read(&mut self) -> E;

    fn read_transform(&mut self, data: E) -> Result<Vec<u8>, ChannelError>
    where
        Self: Sized;

    async fn write(&mut self, data: ChannelWriteData) -> J;

    fn write_transform(&mut self, data: J) -> Result<(), ChannelError>
    where
        Self: Sized;

    async fn flush(&mut self) -> Q;

    async fn close(self);

    fn send_ping() -> bool
    where
        Self: Sized;
}

#[allow(unused)]
pub(crate) enum ChannelError {
    Closed,
    IO(io::Error),
}

#[allow(unused)]
pub struct ChannelInterceptor<
    A: ChannelAdapter<E, J, Q> + Send,
    E: Send + 'static,
    J: Send + 'static,
    Q: Send + 'static,
    C: for<'a> FnMut(&'a mut A, E) -> Pin<Box<dyn Future<Output = E> + Send + 'a>> + Send + 'static,
>(A, C, PhantomData<E>, PhantomData<Q>, PhantomData<J>);

#[allow(unused)]
impl<
        C: for<'a> FnMut(&'a mut A, E) -> Pin<Box<dyn Future<Output = E> + Send + 'a>>
            + Send
            + 'static,
        A: ChannelAdapter<E, J, Q> + Send,
        E: Send + 'static,
        J: Send + 'static,
        Q: Send + 'static,
    > ChannelInterceptor<A, E, J, Q, C>
{
    #[allow(clippy::default_constructed_unit_structs)]
    pub(crate) fn new(state: A, intercept: C) -> Self {
        Self(
            state,
            intercept,
            PhantomData::default(),
            PhantomData::default(),
            PhantomData::default(),
        )
    }
}

#[async_trait]
impl<
        C: for<'a> FnMut(&'a mut A, E) -> Pin<Box<dyn Future<Output = E> + Send + 'a>>
            + Send
            + 'static,
        A: ChannelAdapter<E, J, Q> + Send,
        E: Send + 'static,
        J: Send + 'static,
        Q: Send + 'static,
    > ChannelAdapter<E, J, Q> for ChannelInterceptor<A, E, J, Q, C>
{
    async fn read(&mut self) -> E {
        let result = self.0.read().await;
        self.1(&mut self.0, result).await
    }

    fn read_transform(&mut self, data: E) -> Result<Vec<u8>, ChannelError>
    where
        Self: Sized,
    {
        self.0.read_transform(data)
    }

    async fn write(&mut self, data: ChannelWriteData) -> J {
        self.0.write(data).await
    }

    fn write_transform(&mut self, data: J) -> Result<(), ChannelError>
    where
        Self: Sized,
    {
        self.0.write_transform(data)
    }

    async fn flush(&mut self) -> Q {
        self.0.flush().await
    }

    async fn close(self) {
        self.0.close().await
    }

    fn send_ping() -> bool
    where
        Self: Sized,
    {
        A::send_ping()
    }
}

#[allow(dead_code)]
#[derive(Debug)]
struct SyncIoBridgeCopy<T> {
    src: T,
    rt: tokio::runtime::Handle,
}

pub(crate) trait SyncIOBridgeExt<T> {
    fn inner_mut(&mut self) -> &mut T;
}

impl<T> SyncIOBridgeExt<T> for SyncIoBridge<T> {
    fn inner_mut(&mut self) -> &mut T {
        let r: &mut SyncIoBridgeCopy<T> = unsafe { mem::transmute(self) };
        &mut r.src
    }
}
