use crate::server::stop::ServerStopError;
use crate::server::FetchedServer;
use crate::utils::{ChannelAdapter, ChannelError, ChannelWriteData};
use crate::{ServerContainerWrite, ServerData};
use bollard::Docker;
use ringbuffer::RingBuffer;
use std::ops::DerefMut;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Sender;
use tokio::sync::Mutex;
use tokio::time::Instant;
use tracing::{debug, error, warn};
use uuid::Uuid;

pub(crate) type TerminalSession = (Instant, Arc<AtomicU32>, Arc<AtomicU32>);

pub(crate) struct TerminalData {
    pub(crate) sender: Arc<Sender<ChannelWriteData>>,
    pub(crate) attach_write: Arc<Mutex<Option<ServerContainerWrite>>>,
    pub(crate) cols: Arc<AtomicU32>,
    pub(crate) rows: Arc<AtomicU32>,
    pub(crate) write: bool,
}

pub(crate) async fn new_session<
    A: ChannelAdapter<E, J, Q> + Send + 'static,
    E: Send + 'static,
    J: Send + 'static,
    Q: Send + 'static,
>(
    id: u32,
    mut state: A,
    TerminalData {
        sender,
        attach_write,
        rows,
        cols,
        write,
    }: TerminalData,
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
    state
        .write(ChannelWriteData::Terminal(buffer.to_vec()))
        .await;
    drop(buffer);

    let mut recv = sender.subscribe();
    let mut last_sent = Instant::now();
    let mut ping_sent = false;

    loop {
        if Instant::now().duration_since(last_sent) > Duration::from_secs(10) && A::send_ping() {
            if ping_sent {
                debug!("Closing connection. Terminal session timed out");
                state.close().await;
                let mut sessions = sessions_arc.lock().await;
                sessions.remove(&session_id);
                drop(sessions);
                update_size(id, &docker, server_data_arc.clone()).await;
                return;
            } else {
                let _ = state.write(ChannelWriteData::Ping).await;
                ping_sent = true;
                last_sent = Instant::now();
            }
        }
        tokio::select! {
            stdout = recv.recv() => {
                match stdout {
                    Ok(data) => {
                        let result = state.write(data).await;
                        if let Err(err) = state.write_transform(result) {
                            if let ChannelError::IO(ref io) = err {
                                warn!("Channel disconnected with error: {:?}", io);
                            }
                            drop(err);
                            state.close().await;
                            let mut sessions = sessions_arc.lock().await;
                            sessions.remove(&session_id);
                            drop(sessions);
                            update_size(id, &docker, server_data_arc.clone()).await;
                            return;
                        }
                        state.flush().await;
                    }
                    Err(RecvError::Lagged(_)) => {},
                    Err(RecvError::Closed) => {
                        error!("Closing connection. Internal Sender disconnected");
                        let mut sessions = sessions_arc.lock().await;
                        sessions.remove(&session_id);
                        drop(sessions);
                        update_size(id, &docker, server_data_arc.clone()).await;
                        state.close().await;
                        return;
                    }
                }
            }
            message = state.read() => {
                let message = state.read_transform(message);
                match message {
                    Ok(message) => {
                        last_sent = Instant::now();
                        ping_sent = false;
                        if write {
                            match attach_write.lock().await.deref_mut() {
                                None => {}
                                Some(write) => {
                                    let _ = write.write_all(&message).await;
                                    let _ = write.flush().await;
                                }
                            }
                        }
                    },
                    Err(err) => {
                        if let ChannelError::IO(ref io) = err {
                            warn!("Channel disconnected with error: {:?}", io);
                        }
                        drop(err);
                        state.close().await;
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
