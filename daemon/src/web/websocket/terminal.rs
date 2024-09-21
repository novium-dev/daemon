use crate::terminal::{new_session, update_size, TerminalData};
use crate::utils::ChannelInterceptor;
use crate::web::websocket::{authenticate, WebsocketTerminalQuery};
use crate::{DaemonState, ServerData};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use bollard::Docker;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) async fn websocket(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    query: Query<WebsocketTerminalQuery>,
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
    Ok(ws.on_upgrade(move |socket| {
        terminal_websocket_handle(
            server_id,
            state.docker.clone(),
            server,
            socket,
            Arc::new(AtomicU32::new(query.cols)),
            Arc::new(AtomicU32::new(query.rows)),
        )
    }))
}

const WEBSOCKET_RESIZE_MAGIC: &str = "DAEMON_TTY_RESIZE:";

pub(crate) async fn terminal_websocket_handle(
    id: u32,
    docker: Arc<Docker>,
    server_data_arc: Arc<Mutex<ServerData>>,
    socket: WebSocket,
    cols: Arc<AtomicU32>,
    rows: Arc<AtomicU32>,
) {
    let server_data = server_data_arc.lock().await;
    let sender = server_data.sender.clone();
    let attach_write = server_data.attach_write.clone();
    drop(server_data);

    let server_data_arc2 = server_data_arc.clone();
    let docker2 = docker.clone();
    let cols2 = cols.clone();
    let rows2 = rows.clone();

    tokio::task::spawn(new_session(
        id,
        ChannelInterceptor::new(socket, move |socket, mut message| {
            let server_data_arc2 = server_data_arc2.clone();
            let docker2 = docker2.clone();
            let cols2 = cols2.clone();
            let rows2 = rows2.clone();

            Box::pin(async move {
                if let Some(Ok(Message::Text(data))) = &mut message {
                    if data.starts_with(WEBSOCKET_RESIZE_MAGIC) {
                        let data2 = data[WEBSOCKET_RESIZE_MAGIC.len()..].to_owned();
                        data.clear();
                        if let Some((new_cols, new_rows)) = data2.split_once(':') {
                            let new_cols = new_cols.parse();
                            let new_rows = new_rows.parse();
                            if new_cols.is_err() && new_rows.is_err() {
                                return message;
                            }
                            cols2.store(new_cols.unwrap(), Ordering::Relaxed);
                            rows2.store(new_rows.unwrap(), Ordering::Relaxed);
                            tokio::task::spawn(async move {
                                update_size(id, &docker2, server_data_arc2.clone()).await
                            });
                        }
                    }
                }
                if let Some(Ok(Message::Ping(data))) = &mut message {
                    let _ = socket.send(Message::Pong(std::mem::take(data))).await;
                }
                message
            })
        }),
        TerminalData {
            sender,
            attach_write,
            rows,
            cols,
        },
        docker,
        server_data_arc,
    ));
}
