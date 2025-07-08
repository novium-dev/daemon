use crate::ServerData;
use crate::server::FetchedServer;
use crate::server::status::InternalServerState;
use crate::sync::Mutex;
use crate::utils::{ChannelAdapter, ChannelError, ChannelWriteData};
use axum::extract::ws::WebSocket;
use core::time::Duration;
use sqlx::{MySql, Pool};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::broadcast::Sender;
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, error, warn};

pub(super) async fn stats_websocket_handle(
    id: u32,
    conn: Arc<Pool<MySql>>,
    server_data_arc: Arc<Mutex<ServerData>>,
    socket: WebSocket,
) {
    let server_data = server_data_arc.lock().await;
    let stats_sender = Arc::clone(&server_data.stats_sender);
    drop(server_data);
    tokio::task::spawn(new_stats_session(id, conn, socket, stats_sender));
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
    sender: Arc<Sender<ChannelWriteData>>,
) {
    let mut recv = sender.subscribe();
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
                debug!("Closing connection. Terminal session timed out");
                state.close().await;
                return;
            }
            let _ = state.write(ChannelWriteData::Ping).await;
            ping_sent = true;
            last_sent = Instant::now();
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
