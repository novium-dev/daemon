use crate::schema::FetchedServer;
use crate::schema::{server_buildargs, server_capadd, server_capdrop, server_ports, servers};
use crate::server::{ServerState, ServerStopError};
use crate::storage::StorageIdentifier;
use crate::utils::server;
use crate::web::diesel_error_to_http;
use crate::web::payload::{IdResponse, ServerPayload};
use crate::{schema, storage, DaemonState};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use diesel::dsl::{delete, insert_into, select};
use diesel::{update, Connection, ExpressionMethods, MysqlConnection, RunQueryDsl};
use log::{error, warn};
use std::ops::DerefMut;
use std::time::Duration;
use tokio::time::timeout;

pub(crate) async fn create_server(
    State(state): State<DaemonState>,
    Json(payload): Json<ServerPayload>,
) -> Result<Json<IdResponse>, StatusCode> {
    if payload.resource.is_none() || payload.build.is_none() || payload.ports.is_none() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let id = tokio::task::spawn_blocking(move || {
        let mut conn = state.conn.blocking_lock();

        let id = conn.transaction(|mut conn| {
            let id = insert_dummy(conn.deref_mut())?;
            modify0(conn.deref_mut(), id, &payload)?;
            Ok::<u32, diesel::result::Error>(id)
        })?;
        Ok::<u32, diesel::result::Error>(id)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(diesel_error_to_http)?;

    Ok(Json(IdResponse { id }))
}

pub(crate) async fn modify_server(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Json(payload): Json<ServerPayload>,
) -> Result<(), StatusCode> {
    tokio::task::spawn_blocking(move || {
        let mut conn = state.conn.blocking_lock();
        conn.transaction(|mut conn| {
            modify0(conn.deref_mut(), server_id, &payload)?;
            Ok::<(), diesel::result::Error>(())
        })?;
        Ok::<(), diesel::result::Error>(())
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    .map_err(diesel_error_to_http)?;

    Ok(())
}

pub(crate) async fn delete_server(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<(), StatusCode> {
    let (server, server_data) = server(state.conn.clone(), &state.servers, server_id)
        .await
        .map_err(diesel_error_to_http)?;
    match timeout(Duration::from_secs(10), server_data.lock_owned()).await {
        Ok(mut server_data_locked) => {
            match server
                .kill(&state.docker, server_data_locked.deref_mut())
                .await
            {
                Ok(_) | Err(ServerStopError::NotRunning) => {}
                Err(err) => {
                    warn!("Failed to kill server for deletion: {:?}", err);
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
            }
            let mut conn = state.conn.lock().await;
            conn.transaction(|conn| {
                delete(schema::servers::dsl::servers)
                    .filter(schema::servers::dsl::id.eq(server_id))
                    .execute(conn)?;
                delete(schema::server_capadd::dsl::server_capadd)
                    .filter(schema::server_capadd::dsl::server_id.eq(server_id))
                    .execute(conn)?;
                delete(schema::server_capdrop::dsl::server_capdrop)
                    .filter(schema::server_capdrop::dsl::server_id.eq(server_id))
                    .execute(conn)?;
                delete(schema::server_buildargs::dsl::server_buildargs)
                    .filter(schema::server_buildargs::dsl::server_id.eq(server_id))
                    .execute(conn)?;
                delete(schema::server_ports::dsl::server_ports)
                    .filter(schema::server_ports::dsl::server_id.eq(server_id))
                    .execute(conn)?;

                Ok(())
            })
            .map_err(|err: diesel::result::Error| {
                error!("Failed to commit transaction: {:?}", err);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
            state.servers.lock().await.remove(&server_id);

            tokio::task::spawn(async move {
                let _ = storage::delete(&state.config, StorageIdentifier::Primary(server_id)).await;
            });

            Ok(())
        }
        Err(_) => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}

// ONLY EXECUTE INSIDE A TRANSACTION
pub(crate) fn insert_dummy(conn: &mut MysqlConnection) -> Result<u32, diesel::result::Error> {
    insert_into(servers::dsl::servers)
        .default_values()
        .execute(conn)?;
    Ok(select(schema::last_insert_id()).get_result::<u64>(conn)? as u32)
}

pub(crate) fn modify0(
    conn: &mut MysqlConnection,
    server_id: u32,
    payload: &ServerPayload,
) -> Result<(), diesel::result::Error> {
    let filter = servers::dsl::id.eq(server_id);

    if let Some(resource) = &payload.resource {
        update(servers::dsl::servers)
            .filter(filter)
            .set((
                servers::dsl::cpu.eq(resource.cpu),
                servers::dsl::ram.eq(resource.ram),
                servers::dsl::swap.eq(resource.swap),
                servers::dsl::disk.eq(resource.disk),
            ))
            .execute(conn)?;
    }
    if let Some(build) = &payload.build {
        update(servers::dsl::servers)
            .filter(filter)
            .set((
                servers::dsl::build_identifier.eq(&build.identifier),
                servers::dsl::build_dockerfile.eq(&build.dockerfile),
            ))
            .execute(conn)?;
        delete(server_capadd::dsl::server_capadd)
            .filter(server_capadd::dsl::server_id.eq(server_id))
            .execute(conn)?;
        delete(server_capdrop::dsl::server_capdrop)
            .filter(server_capdrop::dsl::server_id.eq(server_id))
            .execute(conn)?;
        delete(server_buildargs::dsl::server_buildargs)
            .filter(server_buildargs::dsl::server_id.eq(server_id))
            .execute(conn)?;

        for cap in &build.cap_add {
            insert_into(server_capadd::dsl::server_capadd)
                .values((
                    server_capadd::dsl::server_id.eq(server_id),
                    server_capadd::dsl::capability.eq(cap),
                ))
                .execute(conn)?;
        }
        for cap in &build.cap_drop {
            insert_into(server_capdrop::dsl::server_capdrop)
                .values((
                    server_capdrop::dsl::server_id.eq(server_id),
                    server_capdrop::dsl::capability.eq(cap),
                ))
                .execute(conn)?;
        }
        for (key, value) in &build.build_args {
            insert_into(server_buildargs::dsl::server_buildargs)
                .values((
                    server_buildargs::dsl::server_id.eq(server_id),
                    server_buildargs::dsl::key.eq(key),
                    server_buildargs::dsl::value.eq(value),
                ))
                .execute(conn)?;
        }
    }
    if let Some(ports) = &payload.ports {
        delete(server_ports::dsl::server_ports)
            .filter(server_ports::dsl::server_id.eq(server_id))
            .execute(conn)?;
        for port in ports {
            insert_into(server_ports::dsl::server_ports)
                .values((
                    server_ports::dsl::server_id.eq(server_id),
                    server_ports::dsl::internal_port.eq(&port.internal_port),
                    server_ports::dsl::external_port.eq(&port.external_port),
                    server_ports::dsl::external_ip.eq(&port.external_ip),
                    server_ports::dsl::protocol.eq(&port.protocol),
                ))
                .execute(conn)?;
        }
    }

    Ok(())
}

pub(crate) async fn start_server(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<(), StatusCode> {
    let (server, server_data) = server(state.conn.clone(), &state.servers, server_id)
        .await
        .map_err(diesel_error_to_http)?;

    tokio::task::spawn(async move {
        let server_data_locked = timeout(Duration::from_millis(100), server_data.lock()).await;
        if let Ok(mut server_data_locked) = server_data_locked {
            if let Err(err) = server
                .start(
                    &state.config,
                    state.docker.clone(),
                    server_data_locked.deref_mut(),
                    server_data.clone(),
                    state.clone(),
                )
                .await
            {
                warn!("Failed to start server: {:?}", err);
            }
        }
    });

    Ok(())
}

pub(crate) async fn stop_server(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<(), StatusCode> {
    let (server, server_data) = server(state.conn, &state.servers, server_id)
        .await
        .map_err(diesel_error_to_http)?;

    tokio::task::spawn(async move {
        let server_data = timeout(Duration::from_millis(100), server_data.lock()).await;
        if let Ok(mut server_data) = server_data {
            if let Err(err) = server.stop(&state.docker, server_data.deref_mut()).await {
                warn!("Failed to stop server: {:?}", err);
            }
        }
    });

    Ok(())
}

pub(crate) async fn kill_server(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<(), StatusCode> {
    let (server, server_data) = server(state.conn, &state.servers, server_id)
        .await
        .map_err(diesel_error_to_http)?;

    tokio::task::spawn(async move {
        let server_data = timeout(Duration::from_millis(100), server_data.lock()).await;
        if let Ok(mut server_data) = server_data {
            if let Err(err) = server.kill(&state.docker, server_data.deref_mut()).await {
                warn!("Failed to kill server: {:?}", err);
            }
        }
    });

    Ok(())
}

pub(crate) async fn restart_server(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<(), StatusCode> {
    let (server, server_data) = server(state.conn, &state.servers, server_id)
        .await
        .map_err(diesel_error_to_http)?;

    tokio::task::spawn(async move {
        let server_data = timeout(Duration::from_millis(100), server_data.lock()).await;
        if let Ok(mut server_data) = server_data {
            if let Err(err) = server.restart(&state.docker, server_data.deref_mut()).await {
                warn!("Failed to restart server: {:?}", err);
            }
        }
    });

    Ok(())
}

pub(crate) async fn server_info(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<Json<ServerState>, StatusCode> {
    let (server, server_data) = server(state.conn, &state.servers, server_id)
        .await
        .map_err(diesel_error_to_http)?;
    let server_data = timeout(Duration::from_millis(100), server_data.lock()).await;
    match server_data {
        Ok(mut server_data) => Ok(Json(
            FetchedServer::state(server.id, &state.docker, server_data.deref_mut())
                .await
                .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?,
        )),
        Err(_err) => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}
