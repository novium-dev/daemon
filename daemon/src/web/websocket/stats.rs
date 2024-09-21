use crate::web::websocket::{authenticate, WebsocketStatsQuery};
use crate::{DaemonState, ServerData};
use axum::extract::{Path, Query, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use std::sync::Arc;
use tokio::sync::Mutex;

#[allow(unused)]
pub(crate) async fn stats_websocket(
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
    Ok(String::new()) // WIP
}
