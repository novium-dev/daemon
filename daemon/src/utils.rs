// SPDX-License-Identifier: AGPL-3.0-only
use crate::ServerData;
use crate::server::status::InternalServerState;
use crate::server::*;
use async_trait::async_trait;
use cap_async_std::fs::{Dir, MetadataExt as _};
use core::future::Future;
use core::marker::PhantomData;
use core::pin::Pin;
use daemon_common::service::error_server_not_found::FromServerNotFound;
use daemon_common::service::server::{ServerStats, TasksData};
use sha2::{Digest as _, Sha256};
use sqlx::{Executor, MySql, Pool, query_as};
use std::collections::HashMap;
use std::io;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use tokio::sync::Mutex;
use tracing::{error, warn};

/// The IANA timezone name of the host, e.g. `Europe/Berlin`.
///
/// It is resolved once and reused afterwards, so a timezone change on the host
/// only takes effect after a restart of the daemon.
pub fn host_timezone() -> &'static str {
    static TIMEZONE: LazyLock<String> = LazyLock::new(|| {
        iana_time_zone::get_timezone().unwrap_or_else(|err| {
            warn!("Unable to detect the timezone of the host, falling back to UTC: {err:?}");
            "UTC".to_owned()
        })
    });
    &TIMEZONE
}

/// The machine id of the host, which the machine ids of the servers are derived from.
///
/// It is resolved once and reused afterwards.
fn host_machine_id() -> &'static str {
    static MACHINE_ID: LazyLock<String> = LazyLock::new(|| {
        for path in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
            if let Ok(machine_id) = std::fs::read_to_string(path)
                && !machine_id.trim().is_empty()
            {
                return machine_id.trim().to_owned();
            }
        }
        warn!("The host has no machine id, falling back to its hostname");
        sysinfo::System::host_name().unwrap_or_default()
    });
    &MACHINE_ID
}

/// The `/etc/machine-id` of a server, unique per host and server.
///
/// Hytale encrypts its tokens with the machine id, so it has to stay the same across
/// restarts of a server. It is derived from the machine id of the host instead of being
/// random, so that it does not have to be persisted anywhere, comparable to what
/// `sd_id128_get_machine_app_specific` does for application specific ids.
pub fn server_machine_id(server_id: u32) -> String {
    /// A machine id is 16 bytes, written as 32 hexadecimal characters.
    const LENGTH: usize = 16;

    let mut hasher = Sha256::new();
    hasher.update(host_machine_id().as_bytes());
    hasher.update(b"-");
    hasher.update(server_id.to_string().as_bytes());
    hex::encode(
        hasher
            .finalize()
            .into_iter()
            .take(LENGTH)
            .collect::<Vec<u8>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::server_machine_id;

    #[test]
    fn machine_id_has_the_id128_format() {
        let machine_id = server_machine_id(1);

        assert_eq!(machine_id.len(), 32);
        assert!(
            machine_id
                .chars()
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase())
        );

        assert_eq!(machine_id, server_machine_id(1), "not stable");
        assert_ne!(machine_id, server_machine_id(2), "not unique per server");
    }
}

pub fn from_mib(mib: impl Into<u64>) -> u64 {
    mib.into().saturating_mul(0x0010_0000)
}

pub fn to_mib(bytes: impl Into<u64>) -> u64 {
    bytes.into() >> 20u32 // >>20 == /0x0010_0000
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
    servers: &Mutex<HashMap<u32, Arc<ServerData>>>,
    server_id: u32,
) -> Result<Option<(FetchedServer, Arc<ServerData>)>, sqlx::Error> {
    let Some(server) = generic_server_select(conn, server_id).await? else {
        return Ok(None);
    };
    let server_data = Arc::clone(servers.lock().await.entry(server_id).or_default());
    Ok(Some((server, server_data)))
}

pub async fn server_required<T: From<sqlx::Error> + FromServerNotFound>(
    conn: &Pool<MySql>,
    servers: &Mutex<HashMap<u32, Arc<ServerData>>>,
    server_id: u32,
) -> Result<(FetchedServer, Arc<ServerData>), T> {
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
        query_as!(Server, "SELECT `id`,`image`,`cpu`,`ram`,`swap`,`disk`,`storage`,`internet` as `internet: bool`,`ipv6_suffix`,`uid`,`gid`,`group`,`stop_command`,`stop_timeout` FROM `servers` WHERE `id` = ? LIMIT 1", server_id)
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
    Tasks(TasksData),
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

#[derive(Clone)]
pub(crate) struct LinuxOnly<T> {
    #[cfg(target_os = "linux")]
    inner: T,
}

#[cfg(not(target_os = "linux"))]
impl<T> Default for LinuxOnly<T> {
    fn default() -> Self {
        Self {}
    }
}

#[cfg(target_os = "linux")]
impl<T> From<T> for LinuxOnly<T> {
    fn from(value: T) -> Self {
        Self { inner: value }
    }
}

impl<T> Deref for LinuxOnly<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        #[cfg(target_os = "linux")]
        {
            &self.inner
        }

        #[cfg(not(target_os = "linux"))]
        {
            unreachable!()
        }
    }
}

impl<T> DerefMut for LinuxOnly<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        #[cfg(target_os = "linux")]
        {
            &mut self.inner
        }

        #[cfg(not(target_os = "linux"))]
        {
            unreachable!()
        }
    }
}
