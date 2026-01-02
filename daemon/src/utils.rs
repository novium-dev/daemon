// SPDX-License-Identifier: AGPL-3.0-only
use crate::ServerData;
use crate::server::status::InternalServerState;
use crate::server::*;
use async_trait::async_trait;
use cap_async_std::fs::{Dir, MetadataExt as _};
use core::fmt::Debug;
use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use daemon_common::service::error_server_not_found::FromServerNotFound;
use daemon_common::service::server::ServerStats;
use sqlx::{Executor, MySql, Pool, query_as};
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_util::io::SyncIoBridge;
use tracing::error;

pub fn from_mib(mib: impl Into<u64>) -> u64 {
    mib.into().saturating_mul(0x0010_0000)
}

pub async fn database<E: From<sqlx::Error>>(
    conn: impl Executor<'_, Database = MySql> + Copy,
    database_id: u32,
) -> Result<Option<crate::database::FetchedDatabase>, E> {
    let Some(database) = query_as!(
        crate::database::Database,
        "SELECT * FROM `databases` WHERE `id` = ? LIMIT 1",
        database_id
    )
    .fetch_optional(conn)
    .await
    .inspect_err(|err| error!("Unable to resolve database: {:?}", err))?
    else {
        return Ok(None);
    };

    Ok(Some(database.fetched(conn).await.inspect_err(|err| {
        error!("Unable to fetch database: {err:?}");
    })?))
}

pub async fn server(
    conn: &Pool<MySql>,
    servers: &Mutex<HashMap<u32, Arc<Mutex<ServerData>>>>,
    server_id: u32,
) -> Result<Option<(FetchedServer, Arc<Mutex<ServerData>>)>, sqlx::Error> {
    let Some(server) = generic_server_select(conn, server_id).await? else {
        return Ok(None);
    };
    let server_data = Arc::clone(servers.lock().await.entry(server_id).or_default());
    Ok(Some((server, server_data)))
}

pub async fn server_required<T: From<sqlx::Error> + FromServerNotFound>(
    conn: &Pool<MySql>,
    servers: &Mutex<HashMap<u32, Arc<Mutex<ServerData>>>>,
    server_id: u32,
) -> Result<(FetchedServer, Arc<Mutex<ServerData>>), T> {
    server(conn, servers, server_id)
        .await?
        .ok_or_else(<T as FromServerNotFound>::from)
}

pub fn path_relative(str: &str) -> PathBuf {
    if str.starts_with('/') {
        PathBuf::from(format!(".{str}"))
    } else {
        PathBuf::from(format!("./{str}"))
    }
}

pub async fn storage_parent_ino(storage: &Dir, path: &Path) -> Result<u64, io::Error> {
    let path = path.parent().ok_or_else(|| io::Error::other("no parent"))?;
    storage.metadata(path).await.map(|meta| meta.ino())
}

pub async fn generic_server_select(
    conn: impl Executor<'_, Database = MySql> + Copy,
    server_id: u32,
) -> Result<Option<FetchedServer>, sqlx::Error> {
    let server: Option<Server> =
        query_as!(Server, "SELECT `id`,`image`,`cpu`,`ram`,`swap`,`disk`,`storage`,`internet` as `internet: bool`,`ipv6_suffix`,`uid`,`gid`,`stop_command`,`stop_timeout` FROM `servers` WHERE `id` = ? LIMIT 1", server_id)
            .fetch_optional(conn)
            .await?;
    match server {
        Some(server) => server.fetched(conn).await.map(Some),
        None => Ok(None),
    }
}

pub fn parse_path(path: &mut String) -> Result<(Vec<&str>, bool), ()> {
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

pub fn short_path(components: Vec<&str>) -> Result<Vec<&str>, ()> {
    let mut result = Vec::new();
    for component in components {
        if component.eq("..") {
            if result.pop().is_none() {
                return Err(());
            }
        } else if !component.eq(".") {
            result.push(component);
        } else {
            // This is a . component, we just ignore it
        }
    }
    Ok(result)
}

#[derive(Clone)]
pub enum ChannelWriteData {
    Terminal(Vec<u8>),
    Status(InternalServerState),
    Stats(ServerStats),
    Ping,
}

#[async_trait]
pub trait ChannelAdapter<E: Send + 'static, J: Send + 'static, Q: Send + 'static> {
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

pub enum ChannelError {
    Closed,
    IO(io::Error),
}

pub struct ChannelInterceptor<
    A: ChannelAdapter<E, J, Q> + Send,
    E: Send + 'static,
    J: Send + 'static,
    Q: Send + 'static,
    C: for<'a> FnMut(&'a mut A, E) -> Pin<Box<dyn Future<Output = E> + Send + 'a>> + Send + 'static,
>(A, C, PhantomData<E>, PhantomData<Q>, PhantomData<J>);

impl<
    C: for<'a> FnMut(&'a mut A, E) -> Pin<Box<dyn Future<Output = E> + Send + 'a>> + Send + 'static,
    A: ChannelAdapter<E, J, Q> + Send,
    E: Send + 'static,
    J: Send + 'static,
    Q: Send + 'static,
> ChannelInterceptor<A, E, J, Q, C>
{
    pub(crate) const fn new(state: A, intercept: C) -> Self {
        Self(state, intercept, PhantomData, PhantomData, PhantomData)
    }
}

#[async_trait]
impl<
    C: for<'a> FnMut(&'a mut A, E) -> Pin<Box<dyn Future<Output = E> + Send + 'a>> + Send + 'static,
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
        self.0.close().await;
    }

    fn send_ping() -> bool
    where
        Self: Sized,
    {
        A::send_ping()
    }
}

#[derive(Debug)]
#[expect(
    dead_code,
    reason = "this is an exact copy of SyncIoBridge from upstream"
)]
struct SyncIoBridgeCopy<T> {
    src: T,
    rt: tokio::runtime::Handle,
}

pub trait SyncIOBridgeExt<T> {
    fn inner_mut(&mut self) -> &mut T;
}

impl<T> SyncIOBridgeExt<T> for SyncIoBridge<T> {
    fn inner_mut(&mut self) -> &mut T {
        let transmuted = core::ptr::from_mut(self).cast::<SyncIoBridgeCopy<T>>();
        // SAFETY: As long as the crate struct definition doesn't change, this is safe
        unsafe { &mut (*transmuted).src }
    }
}
