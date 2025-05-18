use crate::ServerData;
use crate::sync::Mutex;
use crate::terminal::{TerminalData, new_session};
use crate::utils::ChannelInterceptor;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use bollard::Docker;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;

pub(crate) const WEBSOCKET_RESIZE_MAGIC: &str = "DAEMON_TTY_RESIZE:";

pub(super) async fn terminal_websocket_handle(
    id: u32,
    docker: Arc<Docker>,
    server_data_arc: Arc<Mutex<ServerData>>,
    socket: WebSocket,
    cols: Arc<AtomicU32>,
    rows: Arc<AtomicU32>,
    write: bool,
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
        ChannelInterceptor::new(socket, move |_socket, mut message| {
            #[allow(unused_variables)]
            let server_data_arc2 = server_data_arc2.clone();
            #[allow(unused_variables)]
            let docker2 = docker2.clone();
            #[allow(unused_variables)]
            let cols2 = cols2.clone();
            #[allow(unused_variables)]
            let rows2 = rows2.clone();

            Box::pin(async move {
                if let Some(Ok(Message::Text(data))) = &mut message {
                    let data_str = data.to_string();
                    #[allow(unused_variables)]
                    if let Some(data2) = data_str.strip_prefix(WEBSOCKET_RESIZE_MAGIC) {
                        *data = Utf8Bytes::default();
                        #[cfg(not(feature = "no_tty_size_please"))]
                        if let Some((new_cols, new_rows)) = data2.split_once(':') {
                            use std::sync::atomic::Ordering;

                            let new_cols = new_cols.parse();
                            let new_rows = new_rows.parse();
                            if new_cols.is_err() && new_rows.is_err() {
                                return message;
                            }
                            cols2.store(new_cols.unwrap(), Ordering::Relaxed);
                            rows2.store(new_rows.unwrap(), Ordering::Relaxed);
                            tokio::task::spawn(async move {
                                crate::terminal::update_size(id, &docker2, server_data_arc2.clone())
                                    .await
                            });
                        }
                    }
                }
                message
            })
        }),
        TerminalData {
            sender,
            attach_write,
            rows,
            cols,
            write,
        },
        docker,
        server_data_arc,
    ));
}
