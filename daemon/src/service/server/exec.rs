// SPDX-License-Identifier: AGPL-3.0-only
use crate::ServerData;
use crate::server::FetchedServer;
use crate::service::server::terminal::WEBSOCKET_RESIZE_MAGIC;
use crate::sync::Mutex;
use crate::utils::{ChannelAdapter, ChannelError, ChannelInterceptor, ChannelWriteData};
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use bollard::Docker;
use bollard::container::LogOutput;
use bollard::errors::Error;
use core::pin::Pin;
use core::time::Duration;
use futures::Stream;
use futures_util::StreamExt as _;
use std::sync::Arc;
use tokio::io::{AsyncWrite, AsyncWriteExt as _};
use tokio::time::Instant;
use tracing::{debug, error, warn};

pub(super) async fn exec_websocket_handle(
    server: FetchedServer,
    server_data_arc: Arc<Mutex<ServerData>>,
    docker: Arc<Docker>,
    command: Vec<String>,
    socket: WebSocket,
    cols: u16,
    rows: u16,
) {
    let server_data = server_data_arc.lock().await;
    if let Ok(Some((id, output, input))) = server.exec(&docker, command, &server_data, true).await {
        drop(server_data);

        let _ = FetchedServer::update_exec_tty_size(id.clone(), &docker, cols, rows).await;

        let docker2 = Arc::clone(&docker);
        tokio::task::spawn(new_exec_session(
            ChannelInterceptor::new(socket, move |_socket, mut message| {
                let docker2 = Arc::clone(&docker2);
                let id = id.clone();
                Box::pin(async move {
                    if let Some(Ok(Message::Text(data))) = &mut message {
                        let data_str = data.to_string();
                        if let Some(data2) = data_str.strip_prefix(WEBSOCKET_RESIZE_MAGIC) {
                            *data = Utf8Bytes::default();
                            if let Some((new_cols, new_rows)) = data2.split_once(':') {
                                let Ok(new_cols) = new_cols.parse() else {
                                    return message;
                                };
                                let Ok(new_rows) = new_rows.parse() else {
                                    return message;
                                };
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

#[inline(always)]
async fn new_exec_session<
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
            }
            let _ = state.write(ChannelWriteData::Ping).await;
            ping_sent = true;
            last_sent = Instant::now();
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
