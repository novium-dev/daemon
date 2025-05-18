use crate::ServerData;
use crate::server::FetchedServer;
use crate::service::server::terminal::WEBSOCKET_RESIZE_MAGIC;
use crate::sync::Mutex;
use crate::utils::{ChannelAdapter, ChannelError, ChannelInterceptor, ChannelWriteData};
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use bollard::Docker;
use bollard::container::LogOutput;
use bollard::errors::Error;
use futures::Stream;
use futures_util::StreamExt;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;
use tracing::{debug, error, warn};

pub(super) async fn exec_websocket_handle(
    server: FetchedServer,
    server_data_arc: Arc<Mutex<ServerData>>,
    docker: Arc<Docker>,
    command: Vec<String>,
    socket: WebSocket,
    #[allow(unused_variables)] cols: u32,
    #[allow(unused_variables)] rows: u32,
) {
    let mut server_data = server_data_arc.lock().await;
    if let Ok(opt) = server.exec(&docker, command, &mut server_data, true).await {
        let (id, output, input) = opt.unwrap();
        drop(server_data);

        #[cfg(not(feature = "no_tty_size_please"))]
        let _ = FetchedServer::update_exec_tty_size(id.clone(), &docker, cols, rows).await;

        let docker2 = docker.clone();
        tokio::task::spawn(new_exec_session(
            ChannelInterceptor::new(socket, move |_socket, mut message| {
                #[allow(unused_variables)]
                let docker2 = docker2.clone();
                #[allow(unused_variables)]
                let id = id.clone();
                Box::pin(async move {
                    if let Some(Ok(Message::Text(data))) = &mut message {
                        let data_str = data.to_string();
                        #[allow(unused_variables)]
                        if let Some(data2) = data_str.strip_prefix(WEBSOCKET_RESIZE_MAGIC) {
                            *data = Utf8Bytes::default();
                            #[cfg(not(feature = "no_tty_size_please"))]
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
