use crate::utils::{ChannelAdapter, ChannelError, ChannelWriteData};
use crate::web::websocket::{authenticate, WebsocketStatsQuery};
use crate::{DaemonState, ServerData};
use axum::extract::ws::WebSocket;
use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use log::warn;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Sender;
use tokio::sync::Mutex;
use tokio::time::Instant;

pub(crate) async fn websocket(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    query: Query<WebsocketStatsQuery>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    authenticate(server_id, &state, &query.timestamp, &query.verify).await?;

    let mut servers = state.servers.lock().await;
    let server = match servers.get(&server_id) {
        None => {
            servers.insert(server_id, Arc::new(Mutex::new(ServerData::default())));
            servers.get(&server_id).unwrap()
        }
        Some(server) => server,
    };
    let server = server.clone();
    drop(servers);
    Ok(ws.on_upgrade(move |socket| stats_websocket_handle(server, socket)))
}

pub(crate) async fn stats_websocket_handle(
    server_data_arc: Arc<Mutex<ServerData>>,
    socket: WebSocket,
) {
    let server_data = server_data_arc.lock().await;
    let stats_sender = server_data.stats_sender.clone();
    drop(server_data);
    tokio::task::spawn(new_stats_session(socket, stats_sender));
}

pub(crate) async fn new_stats_session<
    A: ChannelAdapter<E, J, Q> + Send + 'static,
    E: Send + 'static,
    J: Send + 'static,
    Q: Send + 'static,
>(
    mut state: A,
    sender: Arc<Sender<ChannelWriteData>>,
) {
    let mut recv = sender.subscribe();
    let mut last_sent = Instant::now();
    let mut ping_sent = false;

    loop {
        if Instant::now().duration_since(last_sent) > Duration::from_secs(10) && A::send_ping() {
            if ping_sent {
                state.close().await;
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
                            return;
                        }
                        state.flush().await;
                    }
                    Err(RecvError::Lagged(_)) => {},
                    Err(RecvError::Closed) => {
                        state.close().await;
                        return;
                    }
                }
            }
            message = state.read() => {
                let message = state.read_transform(message);
                match message {
                    Ok(_) => {},
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
