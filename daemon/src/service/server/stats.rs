// SPDX-License-Identifier: AGPL-3.0-only
use crate::ServerData;
use crate::server::FetchedServer;
use crate::server::status::InternalServerState;
use crate::sync::task::TaskRwLock;
use crate::utils::{ChannelAdapter, ChannelError, ChannelWriteData, TasksData};
use axum::extract::ws::WebSocket;
use core::time::Duration;
use daemon_common::service::files::task::StorageTask;
use sqlx::{MySql, Pool};
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;
use tracing::{debug, error, warn};

pub(super) async fn stats_websocket_handle(
    id: u32,
    conn: Arc<Pool<MySql>>,
    storage_lock: Arc<TaskRwLock<StorageTask, ()>>,
    server_data: Arc<ServerData>,
    socket: WebSocket,
) {
    tokio::task::spawn(new_stats_session(
        id,
        conn,
        socket,
        storage_lock,
        server_data,
    ));
}

#[inline(always)]
async fn new_stats_session<
    A: ChannelAdapter<E, J, Q> + Send + 'static,
    E: Send + 'static,
    J: Send + 'static,
    Q: Send + 'static,
>(
    id: u32,
    conn: Arc<Pool<MySql>>,
    mut state: A,
    storage_lock: Arc<TaskRwLock<StorageTask, ()>>,
    server_data: Arc<ServerData>,
) {
    let mut recv = server_data.stats_sender.subscribe();
    let mut last_sent = Instant::now();
    let mut ping_sent = false;

    let _ = state
        .write(ChannelWriteData::Status(
            if FetchedServer::resolve_locked(id, &conn).await.is_empty() {
                InternalServerState::Stopped
            } else {
                InternalServerState::Locked
            },
        ))
        .await;

    loop {
        if Instant::now().duration_since(last_sent) > Duration::from_secs(10) && A::send_ping() {
            if ping_sent {
                debug!("Closing connection. Stats session timed out");
                state.close().await;
                return;
            }
            let _ = state.write(ChannelWriteData::Ping).await;
            ping_sent = true;
            last_sent = Instant::now();
        }
        #[rustfmt::skip]
        tokio::select! {
            _timeout = tokio::time::sleep(Duration::from_secs(1)) => {
                let storage_tasks = storage_lock.tasks();
                let server_tasks = server_data.lock.tasks();

                let result = state.write(ChannelWriteData::Tasks(TasksData {
                    storage: storage_tasks,
                    server: server_tasks,
                })).await;
                if let Err(err) = state.write_transform(result) {
                    if let ChannelError::IO(ref err) = err {
                        warn!("Channel disconnected with error: {err:?}");
                    }
                    drop(err);
                    state.close().await;
                    return;
                }
                state.flush().await;
            }
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
                            return;
                         }
                         state.flush().await;
                    }
                    Err(RecvError::Lagged(_)) => {},
                    Err(RecvError::Closed) => {
                        error!("Closing connection. Internal Sender disconnected");
                         state.close().await;
                          return;
                    }
                }
            }
            message = state.read() => {
                let message = state.read_transform(message);
                match message {
                    Ok(_) => {
                        last_sent = Instant::now();
                        ping_sent = false;
                    },
                    Err(err) => {
                        if let ChannelError::IO(ref io) = err {
                            warn!("Channel disconnected with error: {:?}", io);
                        }
                        drop(err);
                        state.close().await;
                        return;
                    }
                }
            }
        };
    }
}
