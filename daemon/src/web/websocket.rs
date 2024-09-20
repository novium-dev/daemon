use crate::terminal::{
    new_session, update_size, ChannelWriteData, SessionError, TerminalAdapter, TerminalData,
};
use crate::{DaemonState, ServerData};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Error;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use bollard::Docker;
use ed25519_dalek::{Signature, Verifier};
use futures_util::SinkExt;
use log::debug;
use serde::Deserialize;
use std::io;
use std::io::ErrorKind;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

#[derive(Deserialize)]
pub(crate) struct WebsocketQuery {
    verify: String,
    timestamp: String,
    cols: u32,
    rows: u32,
}

pub(crate) async fn websocket(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    query: Query<WebsocketQuery>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    debug!("Authenticating WebSocket...");
    let timestamp: u128 = query
        .timestamp
        .parse()
        .map_err(|_err| StatusCode::BAD_REQUEST)?;
    let verify = BASE64_STANDARD
        .decode(query.verify.as_bytes())
        .map_err(|_err| StatusCode::BAD_REQUEST)?;
    let signature_bytes: [u8; 64] = verify.try_into().map_err(|_err| StatusCode::BAD_REQUEST)?;
    state
        .config
        .websocket_key
        .verify(
            format!("{}-{}-{}", state.config.identifier, server_id, timestamp).as_bytes(),
            &Signature::from_bytes(&signature_bytes),
        )
        .map_err(|_err| StatusCode::UNAUTHORIZED)?;
    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_err| StatusCode::BAD_REQUEST)?;
    if (timestamp + state.config.websocket_allow_duration) <= unix.as_millis() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    debug!("WebSocket authentication passed.");

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
        websocket_handle(
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

pub(crate) async fn websocket_handle(
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
        TerminalData {
            sender,
            attach_write,
            rows,
            cols,
        },
        TerminalAdapter {
            state: socket,
            channel_read: move |socket: &mut WebSocket| {
                let server_data_arc2 = server_data_arc2.clone();
                let docker2 = docker2.clone();
                let cols2 = cols2.clone();
                let rows2 = rows2.clone();

                Box::pin(async move {
                    let mut message = socket.recv().await;
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
            },
            channel_read_transform: |message| match message {
                None => Err(SessionError::Closed),
                Some(Err(err)) => Err(SessionError::IO(io::Error::new(ErrorKind::Other, err))),
                Some(Ok(Message::Binary(data))) => Ok(data),
                Some(Ok(Message::Text(text))) => Ok(text.into_bytes()),
                Some(Ok(Message::Close(_))) => Err(SessionError::Closed),
                _ => Ok(Vec::new()),
            },
            channel_write: |socket: &mut WebSocket, data: ChannelWriteData| {
                Box::pin(socket.send(data.into_websocket_message()))
            },
            channel_write_transform: |result: Result<(), Error>| {
                result.map_err(|err| SessionError::IO(io::Error::new(ErrorKind::Other, err)))
            },
            channel_flush: |socket: &mut WebSocket| Box::pin(socket.flush()),
            channel_close: |socket: WebSocket| socket.close(),
            send_ping: true,
        },
        docker,
        server_data_arc,
    ));
}

impl ChannelWriteData {
    pub fn into_websocket_message(self) -> Message {
        match self {
            ChannelWriteData::Terminal(data) => Message::Binary(data),
            ChannelWriteData::Status(state) => state.as_websocket_message(),
            ChannelWriteData::Ping => Message::Ping(Vec::new()),
            ChannelWriteData::Stats(stats) => stats.as_websocket_message(),
        }
    }
}
