use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::sync::Mutex;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use bollard::Docker;

use crate::terminal::{TerminalData, new_session, update_size};
use crate::utils::ChannelInterceptor;
use crate::web::user::authenticate_user;
use crate::web::websocket::WebsocketTerminalQuery;
use crate::{Daemon, ServerData};

#[utoipa::path(get,
    tag = "server",
    path = "/user/server/{server_id}/terminal",
    description = "A websocket for interaction with the terminal (User Request)",
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
        ("verify" = String, Query, description = "The signature to proof that the user request was permitted"),
        ("cols" = u32, Query, description = "The initial value of columns in the terminal"),
        ("rows" = u32, Query, description = "The initial value of rows in the terminal"),
        ("write" = bool, Query, description = "If write access to the terminal should be granted")
   )
)]
pub(crate) async fn websocket(
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
    query: Query<WebsocketTerminalQuery>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    let write = query.write;
    authenticate_user(
        "terminal",
        server_id,
        &state,
        &query.timestamp,
        &query.verify,
        Some(format!("-{}", write)),
    )
    .await?;

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
            write,
        )
    }))
}

pub(crate) const WEBSOCKET_RESIZE_MAGIC: &str = "DAEMON_TTY_RESIZE:";

pub(crate) async fn terminal_websocket_handle(
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
            let server_data_arc2 = server_data_arc2.clone();
            let docker2 = docker2.clone();
            let cols2 = cols2.clone();
            let rows2 = rows2.clone();

            Box::pin(async move {
                if let Some(Ok(Message::Text(data))) = &mut message {
                    let data_str = data.to_string();
                    if let Some(data2) = data_str.strip_prefix(WEBSOCKET_RESIZE_MAGIC) {
                        *data = Utf8Bytes::default();
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
