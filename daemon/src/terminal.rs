use crate::schema::FetchedServer;
use crate::server::{ServerState, ServerStats, ServerStopError};
use crate::{ServerContainerWrite, ServerData};
use bollard::Docker;
use log::warn;
use std::future::Future;
use std::io;
use std::ops::DerefMut;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Sender;
use tokio::sync::Mutex;
use tokio::time::Instant;
use uuid::Uuid;

pub(crate) type TerminalSession = (Instant, Arc<AtomicU32>, Arc<AtomicU32>);

pub(crate) enum SessionError {
    Closed,
    IO(io::Error),
}

#[derive(Clone)]
pub(crate) enum ChannelWriteData {
    Terminal(Vec<u8>),
    Status(ServerState),
    Stats(ServerStats),
    Ping,
}

pub(crate) struct TerminalAdapter<
    A: Send + 'static,
    R: for<'a> FnMut(&'a mut A) -> Pin<Box<dyn Future<Output = E> + 'a + Send>> + Send + 'static,
    E: Send + 'static,
    F: FnMut(E) -> Result<Vec<u8>, SessionError> + Send + 'static,
    G: for<'a> FnMut(&'a mut A, ChannelWriteData) -> Pin<Box<dyn Future<Output = I> + Send + 'a>>
        + Send
        + 'static,
    I: Send + 'static,
    J: FnMut(I) -> Result<(), SessionError> + Send + 'static,
    K: FnOnce(A) -> M + Send + 'static,
    M: Future<Output = P> + Send + 'static,
    N: for<'a> FnMut(&'a mut A) -> Pin<Box<dyn Future<Output = Q> + Send + 'a>> + Send + 'static,
    P: Send + 'static,
    Q: Send + 'static,
> {
    pub(crate) state: A,
    pub(crate) channel_read: R,
    pub(crate) channel_read_transform: F,
    pub(crate) channel_write: G,
    pub(crate) channel_write_transform: J,
    pub(crate) channel_flush: N,
    pub(crate) channel_close: K,
    pub(crate) send_ping: bool,
}

pub(crate) struct TerminalData {
    pub(crate) sender: Arc<Sender<ChannelWriteData>>,
    pub(crate) attach_write: Arc<Mutex<Option<ServerContainerWrite>>>,
    pub(crate) cols: Arc<AtomicU32>,
    pub(crate) rows: Arc<AtomicU32>,
}

pub(crate) async fn new_session<
    A: Send + 'static,
    R: for<'a> FnMut(&'a mut A) -> Pin<Box<dyn Future<Output = E> + 'a + Send>> + Send + 'static,
    E: Send + 'static,
    F: FnMut(E) -> Result<Vec<u8>, SessionError> + Send + 'static,
    G: for<'a> FnMut(&'a mut A, ChannelWriteData) -> Pin<Box<dyn Future<Output = I> + Send + 'a>>
        + Send
        + 'static,
    I: Send + 'static,
    J: FnMut(I) -> Result<(), SessionError> + Send + 'static,
    K: FnOnce(A) -> M + Send + 'static,
    M: Future<Output = P> + Send + 'static,
    N: for<'a> FnMut(&'a mut A) -> Pin<Box<dyn Future<Output = Q> + Send + 'a>> + Send + 'static,
    P: Send + 'static,
    Q: Send + 'static,
>(
    id: u32,
    TerminalData {
        sender,
        attach_write,
        rows,
        cols,
    }: TerminalData,
    TerminalAdapter {
        mut state,
        mut channel_read,
        mut channel_read_transform,
        mut channel_write,
        mut channel_write_transform,
        mut channel_flush,
        channel_close,
        send_ping,
    }: TerminalAdapter<A, R, E, F, G, I, J, K, M, N, P, Q>,
    docker: Arc<Docker>,
    server_data_arc: Arc<Mutex<ServerData>>,
) {
    let server_data = server_data_arc.lock().await;
    let buffer = server_data.buffer.clone();
    let sessions_arc = server_data.sessions.clone();
    drop(server_data);
    let mut sessions = sessions_arc.lock().await;
    let session_id = Uuid::new_v4();
    sessions.insert(session_id, (Instant::now(), cols.clone(), rows.clone()));
    drop(sessions);
    update_size(id, &docker, server_data_arc.clone()).await;
    let buffer = buffer.lock().await;
    channel_write(&mut state, ChannelWriteData::Terminal(buffer.clone())).await;
    drop(buffer);

    let _ = channel_write(&mut state, ChannelWriteData::Stats(ServerStats::default())).await;
    let _ = channel_flush(&mut state).await;

    let docker2 = docker.clone();
    let server_data_arc2 = server_data_arc.clone();
    let mut recv = sender.subscribe();
    tokio::task::spawn(async move {
        FetchedServer::update_status(id, &sender, &docker2, &server_data_arc2).await;
    });
    let mut last_sent = Instant::now();
    let mut ping_sent = false;

    loop {
        if Instant::now().duration_since(last_sent) > Duration::from_secs(10) && send_ping {
            if ping_sent {
                channel_close(state).await;
                let mut sessions = sessions_arc.lock().await;
                sessions.remove(&session_id);
                drop(sessions);
                update_size(id, &docker, server_data_arc.clone()).await;
                return;
            } else {
                let _ = channel_write(&mut state, ChannelWriteData::Ping).await;
                ping_sent = true;
                last_sent = Instant::now();
            }
        }
        tokio::select! {
            stdout = recv.recv() => {
                match stdout {
                    Ok(data) => {
                        if let Err(err) = channel_write_transform(channel_write(&mut state, data).await) {
                            if let SessionError::IO(ref io) = err {
                                warn!("Channel disconnected with error: {:?}", io);
                            }
                            drop(err);
                            channel_close(state).await;
                            let mut sessions = sessions_arc.lock().await;
                            sessions.remove(&session_id);
                            drop(sessions);
                            update_size(id, &docker, server_data_arc.clone()).await;
                            return;
                        }
                        channel_flush(&mut state).await;
                    }
                    Err(RecvError::Lagged(_)) => {},
                    Err(RecvError::Closed) => {
                        channel_close(state).await;
                        return;
                    }
                }
            }
            message = channel_read(&mut state) => {
                let message = channel_read_transform(message);
                match message {
                    Ok(message) => {
                        last_sent = Instant::now();
                        ping_sent = false;
                        match attach_write.lock().await.deref_mut() {
                            None => {}
                            Some(write) => {
                                let _ = write.write_all(&message).await;
                                let _ = write.flush().await;
                            }
                        }
                    },
                    Err(err) => {
                        if let SessionError::IO(ref io) = err {
                            warn!("Channel disconnected with error: {:?}", io);
                        }
                        drop(err);
                        channel_close(state).await;
                        let mut sessions = sessions_arc.lock().await;
                        sessions.remove(&session_id);
                        drop(sessions);
                        update_size(id, &docker, server_data_arc.clone()).await;
                        return;
                    }
                }
            }
        };
    }
}

pub(crate) async fn update_size(id: u32, docker: &Docker, server_data: Arc<Mutex<ServerData>>) {
    let mut server_data = server_data.lock().await;
    let sessions = server_data.sessions.lock().await;
    let mut sessions_vec: Vec<_> = sessions.iter().collect();
    sessions_vec.sort_by_key(|(_id, (time, _cols, _rows))| time);
    if let Some((_id, (_time, cols, rows))) = sessions_vec.into_iter().next() {
        let cols = cols.load(Ordering::Relaxed);
        let rows = rows.load(Ordering::Relaxed);
        drop(sessions);
        if server_data.size.0 != cols || server_data.size.1 != rows {
            if let Err(err) =
                FetchedServer::update_tty_size(id, docker, &mut server_data, cols, rows).await
            {
                if let ServerStopError::NotRunning = err {
                    return;
                }
                warn!("Failed to resize tty to {},{}: {:?}", cols, rows, err);
            }
        }
    }
}
