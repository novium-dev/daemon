/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
use crate::ServerData;
use crate::sync::Mutex;
use crate::terminal::{TerminalData, new_session};
use crate::utils::ChannelInterceptor;
use axum::extract::ws::{Message, Utf8Bytes};
use axum::extract::ws::WebSocket;
#[cfg(not(feature = "no_tty_size_please"))]
use bollard::Docker;
#[cfg(not(feature = "no_tty_size_please"))]
use core::sync::atomic::AtomicU16;
use std::sync::Arc;

pub const WEBSOCKET_RESIZE_MAGIC: &str = "DAEMON_TTY_RESIZE:";

pub(super) async fn terminal_websocket_handle(
    id: u32,
    #[cfg(not(feature = "no_tty_size_please"))]
    docker: Arc<Docker>,
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
    #[cfg(feature = "no_tty_size_please")]
    let _ = id;

    #[cfg(not(feature = "no_tty_size_please"))]
    let server_data_arc2 = Arc::clone(&server_data_arc);
    #[cfg(not(feature = "no_tty_size_please"))]
    let docker2 = Arc::clone(&docker);
    #[cfg(not(feature = "no_tty_size_please"))]
    let cols2 = Arc::clone(&cols);
    #[cfg(not(feature = "no_tty_size_please"))]
    let rows2 = Arc::clone(&rows);
    tokio::task::spawn(new_session(
        #[cfg(not(feature = "no_tty_size_please"))]
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
            #[cfg(not(feature = "no_tty_size_please"))]
            cols,
            #[cfg(not(feature = "no_tty_size_please"))]
            rows,
            write,
        },
        #[cfg(not(feature = "no_tty_size_please"))]
        docker,
        server_data_arc,
    ));
}
