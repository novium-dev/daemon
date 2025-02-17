use crate::sync::Mutex;
use async_std::prelude::Stream;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use bollard::container::LogOutput;
use bollard::errors::Error;
use bollard::Docker;
use futures_util::StreamExt;
use serde::Deserialize;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;
use tracing::{debug, error, warn};

use crate::server::FetchedServer;
use crate::utils::{server, ChannelAdapter, ChannelError, ChannelInterceptor, ChannelWriteData};
use crate::web::sql_error_to_http;
use crate::web::user::authenticate_user;
use crate::{DaemonState, ServerData};

#[derive(Debug, Deserialize)]
pub(crate) struct ExecUserQuery {
    pub(crate) verify: String,
    pub(crate) timestamp: String,
    pub(crate) command: String,
}

#[utoipa::path(get,
    tag = "server",
    path = "/user/server/{server_id}/exec",
    description = "A websocket for server execution (User Request)",
    responses(
        (status = 200, description = "Server does exist"),
        (status = 404, description = "Server does not exist"),
        (status = 503, description = "Server can't be accessed right now"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "The proof of the permission for the user request is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("timestamp" = u64, Query, description = "The time the user request got permitted as Utc timestamp (milliseconds)"),
        ("verify" = String, Query, description = "The signature to proof that the user request was permitted")
   )
)]
pub(crate) async fn websocket(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    query: Query<ExecUserQuery>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    authenticate_user(
        "exec",
        server_id,
        &state,
        &query.timestamp,
        &query.verify,
        Some(format!("-{}", query.command)),
    )
    .await?;
    let command: Vec<String> =
        serde_json::from_str(&query.command).map_err(|_err| StatusCode::BAD_REQUEST)?;

    let Some((server, server_data)) = server(state.conn.deref(), &state.servers, server_id)
        .await
        .map_err(sql_error_to_http)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };

    Ok(ws.on_upgrade(move |socket| {
        exec_websocket_handle(server, server_data, state.docker.clone(), command, socket)
    }))
}

pub(crate) async fn exec_websocket_handle(
    server: FetchedServer,
    server_data_arc: Arc<Mutex<ServerData>>,
    docker: Arc<Docker>,
    command: Vec<String>,
    socket: WebSocket,
) {
    let mut server_data = server_data_arc.lock().await;
    if let Ok(opt) = server.exec(&docker, command, &mut server_data, true).await {
        let (id, output, input) = opt.unwrap();
        drop(server_data);
        let docker2 = docker.clone();
        tokio::task::spawn(new_exec_session(
            ChannelInterceptor::new(socket, move |_socket, mut message| {
                let docker2 = docker2.clone();

                let id = id.clone();
                Box::pin(async move {
                    if let Some(Ok(Message::Text(data))) = &mut message {
                        let data_str = data.to_string();
                        if let Some(data2) = data_str
                            .strip_prefix(crate::web::websocket::terminal::WEBSOCKET_RESIZE_MAGIC)
                        {
                            *data = Utf8Bytes::default();
                            if let Some((new_cols, new_rows)) = data2.split_once(':') {
                                let new_cols = new_cols.parse();
                                let new_rows = new_rows.parse();
                                if new_cols.is_err() && new_rows.is_err() {
                                    return message;
                                }
                                let new_cols = new_cols.unwrap();
                                let new_rows = new_rows.unwrap();
                                tokio::task::spawn(async move {
                                    let _ = FetchedServer::update_exec_tty_size(
                                        id.clone(),
                                        &docker2,
                                        new_cols,
                                        new_rows,
                                    )
                                    .await;
                                });
                            }
                        }
                    }
                    message
                })
            }),
            output,
            input,
        ));
    }
}

pub(crate) async fn new_exec_session<
    A: ChannelAdapter<E, J, Q> + Send + 'static,
    E: Send + 'static,
    J: Send + 'static,
    Q: Send + 'static,
>(
    mut state: A,
    mut output: Pin<Box<dyn Stream<Item = Result<LogOutput, Error>> + Send>>,
    mut input: Pin<Box<dyn AsyncWrite + Send>>,
) {
    let mut last_sent = Instant::now();
    let mut ping_sent = false;

    loop {
        if Instant::now().duration_since(last_sent) > Duration::from_secs(10) && A::send_ping() {
            if ping_sent {
                debug!("Closing connection. Terminal session timed out");
                state.close().await;
                return;
            } else {
                let _ = state.write(ChannelWriteData::Ping).await;
                ping_sent = true;
                last_sent = Instant::now();
            }
        }
        tokio::select! {
            stdout = output.next() => {
                match stdout {
                    Some(Ok(data)) => {
                        let result = state.write(ChannelWriteData::Terminal(data.into_bytes().to_vec())).await;
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
                    Some(err) => {
                        error!("Closing connection. Docker stream error: {:?}", err);
                        state.close().await;
                        return;
                    },
                    None => {
                        debug!("Closing connection. Docker stream disconnected");
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
                        let _ = input.write_all(&message).await;
                        let _ = input.flush().await;
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
