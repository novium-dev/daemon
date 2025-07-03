use crate::ServerData;
use crate::sync::Mutex;
use crate::terminal::{TerminalData, new_session};
use crate::utils::ChannelInterceptor;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use bollard::Docker;
use core::sync::atomic::AtomicU16;
use std::sync::Arc;

pub const WEBSOCKET_RESIZE_MAGIC: &str = "DAEMON_TTY_RESIZE:";

pub(super) async fn terminal_websocket_handle(
    id: u32,
    #[cfg(not(feature = "no_tty_size_please"))] docker: Arc<Docker>,
    server_data_arc: Arc<Mutex<ServerData>>,
    socket: WebSocket,
    #[cfg(not(feature = "no_tty_size_please"))] cols: Arc<AtomicU16>,
    #[cfg(not(feature = "no_tty_size_please"))] rows: Arc<AtomicU16>,
    write: bool,
) {
    let server_data = server_data_arc.lock().await;
    let sender = Arc::clone(&server_data.sender);
    let attach_write = Arc::clone(&server_data.attach_write);
    drop(server_data);

    #[cfg(not(feature = "no_tty_size_please"))]
    let server_data_arc2 = Arc::clone(&server_data_arc);
    #[cfg(not(feature = "no_tty_size_please"))]
    let docker2 = Arc::clone(&docker);
    #[cfg(not(feature = "no_tty_size_please"))]
    let cols2 = Arc::clone(&cols);
    #[cfg(not(feature = "no_tty_size_please"))]
    let rows2 = Arc::clone(&rows);
    #[cfg(not(feature = "no_tty_size_please"))]
    tokio::task::spawn(new_session(
        id,
        ChannelInterceptor::new(socket, move |_socket, mut message| {
            #[cfg(not(feature = "no_tty_size_please"))]
            let server_data_arc2 = Arc::clone(&server_data_arc2);
            #[cfg(not(feature = "no_tty_size_please"))]
            let docker2 = Arc::clone(&docker2);
            #[cfg(not(feature = "no_tty_size_please"))]
            let cols2 = Arc::clone(&cols2);
            #[cfg(not(feature = "no_tty_size_please"))]
            let rows2 = Arc::clone(&rows2);

            Box::pin(async move {
                if let Some(Ok(Message::Text(data))) = &mut message {
                    let data_str = data.to_string();
                    #[cfg_attr(
                        feature = "no_tty_size_please",
                        expect(unused_variables, reason = "unused data2")
                    )]
                    if let Some(data2) = data_str.strip_prefix(WEBSOCKET_RESIZE_MAGIC) {
                        *data = Utf8Bytes::default();
                        #[cfg(not(feature = "no_tty_size_please"))]
                        if let Some((new_cols, new_rows)) = data2.split_once(':') {
                            use core::sync::atomic::Ordering;

                            let Ok(new_cols) = new_cols.parse() else {
                                return message;
                            };
                            let Ok(new_rows) = new_rows.parse() else {
                                return message;
                            };
                            cols2.store(new_cols, Ordering::Relaxed);
                            rows2.store(new_rows, Ordering::Relaxed);
                            tokio::task::spawn(async move {
                                crate::terminal::update_size(
                                    id,
                                    &docker2,
                                    Arc::clone(&server_data_arc2),
                                )
                                .await;
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
            cols,
            rows,
            write,
        },
        docker,
        server_data_arc,
    ));
}
