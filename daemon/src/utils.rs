#[cfg(feature = "database")]
use crate::schema::{databases, Database, FetchedDatabase};
use crate::schema::{servers, FetchedServer, Server};
use crate::server::InternalServerState;
use crate::ServerData;
use axum::async_trait;
use daemon_common::server::ServerStats;
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::{AsyncMysqlConnection, RunQueryDsl};
use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::io::{Error, ErrorKind};
use std::marker::PhantomData;
use std::pin::{pin, Pin};
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::Mutex;
#[cfg(feature = "database")]
use tokio::sync::OwnedMutexGuard;

pub(crate) fn from_mib(mib: u32) -> i64 {
    mib as i64 * 1024 * 1024
}

#[cfg(feature = "database")]
pub(crate) async fn database(
    mut conn: OwnedMutexGuard<AsyncMysqlConnection>,
    database_id: u32,
) -> Result<(FetchedDatabase, OwnedMutexGuard<AsyncMysqlConnection>), diesel::result::Error> {
    let database = databases::dsl::databases
        .filter(databases::dsl::id.eq(database_id))
        .select(Database::as_select())
        .first(&mut conn)
        .await?;
    Ok((database.fetched(&mut conn).await?, conn))
}

pub(crate) async fn server(
    conn: &mut AsyncMysqlConnection,
    servers: &Mutex<HashMap<u32, Arc<Mutex<ServerData>>>>,
    server_id: u32,
) -> Result<(FetchedServer, Arc<Mutex<ServerData>>), diesel::result::Error> {
    let server = generic_server_select(conn, server_id).await?;

    let mut servers = servers.lock().await;
    servers
        .entry(server_id)
        .or_insert_with(|| Arc::new(Mutex::new(ServerData::default())));
    Ok((server, servers.get(&server_id).unwrap().clone()))
}

pub(crate) async fn generic_server_select(
    conn: &mut AsyncMysqlConnection,
    server_id: u32,
) -> Result<FetchedServer, diesel::result::Error> {
    let server = servers::dsl::servers
        .filter(servers::dsl::id.eq(server_id))
        .select(Server::as_select())
        .first(conn)
        .await?;
    server.fetched(conn).await
}

#[cfg(any(feature = "sftp", feature = "files"))]
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

#[cfg(any(feature = "sftp", feature = "files"))]
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

#[allow(unused)]
pub(crate) struct ChannelStreamReceiver(pub(crate) Receiver<([u8; 4096], usize)>);
#[allow(unused)]
pub(crate) struct ChannelStreamSender(pub(crate) Option<Sender<([u8; 4096], usize)>>);

impl AsyncRead for ChannelStreamReceiver {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.0.poll_recv(cx).map(|opt| match opt {
            None => Ok(()),
            Some((buffer, len)) => {
                buf.set_filled(len);
                buf.filled_mut().clone_from_slice(&buffer[0..len]);
                Ok(())
            }
        })
    }
}

impl ChannelStreamSender {
    async fn write(self: Pin<&mut Self>, buf: &[u8]) -> Result<usize, Error> {
        match self.0 {
            None => Ok(0),
            Some(ref sender) => {
                let mut written = 0;
                while written < buf.len() {
                    let mut buffer = [0; 4096];
                    let len = if (buf.len() - written) <= 4096 {
                        buf.len() - written
                    } else {
                        4096
                    };
                    buffer.clone_from_slice(&buf[written..(written + len)]);
                    sender
                        .send((buffer, len))
                        .await
                        .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                    written += len;
                }
                Ok(written)
            }
        }
    }
}

impl AsyncWrite for ChannelStreamSender {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, Error>> {
        use futures_util::Future;
        let mut pinned = pin!(self.write(buf));
        pinned.as_mut().poll(cx)
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.0.take();
        Poll::Ready(Ok(()))
    }
}
