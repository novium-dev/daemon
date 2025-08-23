/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
#[cfg(not(feature = "no_tty_size_please"))]
use crate::server::FetchedServer;
#[cfg(not(feature = "no_tty_size_please"))]
use crate::server::stop::ServerStopError;
use crate::sync::Mutex;
use crate::utils::{ChannelAdapter, ChannelError, ChannelWriteData};
use crate::{ServerContainerWrite, ServerData};
#[cfg(not(feature = "no_tty_size_please"))]
use bollard::Docker;
#[cfg(not(feature = "no_tty_size_please"))]
use core::sync::atomic::AtomicU16;
use core::time::Duration;
use ringbuffer::RingBuffer as _;
use std::sync::Arc;
use tokio::io::AsyncWriteExt as _;
use tokio::sync::broadcast::Sender;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;
use tracing::{debug, error, warn};

#[cfg(not(feature = "no_tty_size_please"))]
pub struct TerminalSession {
    pub time: Instant,
    pub cols: Arc<AtomicU16>,
    pub rows: Arc<AtomicU16>,
}

pub struct TerminalData {
    pub sender: Arc<Sender<ChannelWriteData>>,
    pub attach_write: Arc<Mutex<Option<ServerContainerWrite>>>,
    #[cfg(not(feature = "no_tty_size_please"))]
    pub cols: Arc<AtomicU16>,
    #[cfg(not(feature = "no_tty_size_please"))]
    pub rows: Arc<AtomicU16>,
    pub write: bool,
}

pub async fn new_session<
    A: ChannelAdapter<E, J, Q> + Send + 'static,
    E: Send + 'static,
    J: Send + 'static,
    Q: Send + 'static,
>(
    #[cfg(not(feature = "no_tty_size_please"))] id: u32,
    mut state: A,
    TerminalData {
        sender,
        attach_write,
        #[cfg(not(feature = "no_tty_size_please"))]
        rows,
        #[cfg(not(feature = "no_tty_size_please"))]
        cols,
        write,
    }: TerminalData,
    #[cfg(not(feature = "no_tty_size_please"))] docker: Arc<Docker>,
    server_data_arc: Arc<Mutex<ServerData>>,
) {
    let server_data = server_data_arc.lock().await;
    let buffer = Arc::clone(&server_data.buffer);
    #[cfg(not(feature = "no_tty_size_please"))]
    let (sessions_arc, session_id) = {
        let sessions_arc = Arc::clone(&server_data.sessions);
        drop(server_data);
        let mut sessions = sessions_arc.lock().await;
        let session_id = uuid::Uuid::new_v4();
        sessions.insert(
            session_id,
            TerminalSession {
                time: Instant::now(),
                cols: Arc::clone(&cols),
                rows: Arc::clone(&rows),
            },
        );
        drop(sessions);
        update_size(id, &docker, Arc::clone(&server_data_arc)).await;
        (sessions_arc, session_id)
    };
    #[cfg(feature = "no_tty_size_please")]
    {
        drop(server_data);
    };

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
                #[cfg(not(feature = "no_tty_size_please"))]
                {
                    let mut sessions = sessions_arc.lock().await;
                    sessions.remove(&session_id);
                    drop(sessions);
                    update_size(id, &docker, Arc::clone(&server_data_arc)).await;
                };
                return;
            }
            let _ = state.write(ChannelWriteData::Ping).await;
            ping_sent = true;
            last_sent = Instant::now();
        }
        #[expect(
            clippy::integer_division_remainder_used,
            reason = "probably caused by tokio macro"
        )]
        {
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
                                #[cfg(not(feature = "no_tty_size_please"))]
                                {
                                    let mut sessions = sessions_arc.lock().await;
                                    sessions.remove(&session_id);
                                    drop(sessions);
                                    update_size(id, &docker, Arc::clone(&server_data_arc)).await;
                                };
                                return;
                            }
                            state.flush().await;
                        }
                        Err(RecvError::Lagged(_)) => {},
                        Err(RecvError::Closed) => {
                            error!("Closing connection. Internal Sender disconnected");
                            #[cfg(not(feature = "no_tty_size_please"))]
                            {
                                let mut sessions = sessions_arc.lock().await;
                                sessions.remove(&session_id);
                                drop(sessions);
                                update_size(id, &docker, Arc::clone(&server_data_arc)).await;
                            };
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
                                match *attach_write.lock().await {
                                    None => {}
                                    Some(ref mut write) => {
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
                            #[cfg(not(feature = "no_tty_size_please"))]
                            {
                                let mut sessions = sessions_arc.lock().await;
                                sessions.remove(&session_id);
                                drop(sessions);
                                update_size(id, &docker, Arc::clone(&server_data_arc)).await;
                            };
                            return;
                        }
                    }
                }
            };
        };
    }
}

#[cfg(not(feature = "no_tty_size_please"))]
pub async fn update_size(id: u32, docker: &Docker, server_data: Arc<Mutex<ServerData>>) {
    let mut server_data = server_data.lock().await;
    let sessions = server_data.sessions.lock().await;
    let mut sessions_vec: Vec<_> = sessions.iter().collect();
    sessions_vec.sort_by_key(|(_id, TerminalSession { time, .. })| time);
    for (_id, TerminalSession { cols, rows, .. }) in sessions_vec {
        use core::sync::atomic::Ordering;
        let cols = cols.load(Ordering::Relaxed);
        let rows = rows.load(Ordering::Relaxed);
        if cols == 0 || rows == 0 {
            continue;
        }

        drop(sessions);
        if server_data.size.0 != cols || server_data.size.1 != rows {
            if let Err(err) =
                FetchedServer::update_tty_size(id, docker, &mut server_data, cols, rows).await
            {
                if matches!(err, ServerStopError::NotRunning) {
                    return;
                }
                warn!("Failed to resize tty to {},{}: {:?}", cols, rows, err);
            }
        }
        drop(server_data);
        break;
    }
}
