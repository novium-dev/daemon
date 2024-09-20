use crate::schema::{servers, FetchedServer, Server};
use crate::ServerData;
use diesel::{ExpressionMethods, MysqlConnection, QueryDsl, RunQueryDsl, SelectableHelper};
use std::collections::HashMap;
use std::ops::DerefMut;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) fn from_mib(mib: u32) -> i64 {
    mib as i64 * 1024 * 1024
}

pub(crate) async fn server(
    conn: Arc<Mutex<MysqlConnection>>,
    servers: &Mutex<HashMap<u32, Arc<Mutex<ServerData>>>>,
    server_id: u32,
) -> Result<(FetchedServer, Arc<Mutex<ServerData>>), diesel::result::Error> {
    let server = generic_server_select(conn, server_id).await?;

    let mut servers = servers.lock().await;
    servers
        .entry(server_id)
        .or_insert_with(|| Arc::new(Mutex::new(ServerData::default())));
    Ok((server, servers.get(&server_id).unwrap().clone()))
}

pub(crate) async fn generic_server_select(
    conn: Arc<Mutex<MysqlConnection>>,
    server_id: u32,
) -> Result<FetchedServer, diesel::result::Error> {
    tokio::task::spawn_blocking(move || {
        let mut conn = conn.blocking_lock();
        let server = servers::dsl::servers
            .filter(servers::dsl::id.eq(server_id))
            .select(Server::as_select())
            .first(conn.deref_mut())?;
        server.fetched(conn.deref_mut())
    })
    .await
    .unwrap()
}

pub(crate) fn short_path(components: Vec<&str>) -> Result<Vec<&str>, ()> {
    let mut result = Vec::new();
    for component in components {
        if component.eq("..") {
            if !result.is_empty() {
                result.remove(result.len() - 1);
            } else {
                return Err(());
            }
        } else {
            result.push(component);
        }
    }
    Ok(result)
}
