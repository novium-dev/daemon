use std::ops::DerefMut;
use std::time::Duration;

use crate::common::payload::server::ServerPayload;
use crate::common::payload::IdResponse;
use crate::schema::*;
use crate::server::{InternalServerState, ServerStartError, ServerStopError};
use crate::storage::StorageIdentifier;
use crate::utils::{server, storage_read};
use crate::web::diesel_error_to_http;
use crate::{schema, storage, DaemonState};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use daemon_common::payload::server::ServerFileWrite;
use diesel::dsl::{delete, insert_into, select};
use diesel::{update, ExpressionMethods};
use diesel_async::{AsyncConnection, AsyncMysqlConnection, RunQueryDsl};
use log::{debug, error, warn};
use tokio::time::timeout;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub(crate) fn config() -> OpenApiRouter<DaemonState> {
    OpenApiRouter::new()
        .routes(routes!(create_server))
        .routes(routes!(modify_server))
        .routes(routes!(delete_server))
        .routes(routes!(server_status))
        .routes(routes!(start_server))
        .routes(routes!(stop_server))
        .routes(routes!(kill_server))
        .routes(routes!(restart_server))
}

#[utoipa::path(put,
    path = "/server",
    tag = "server",
    description = "Creates a new server",
    request_body(
        content = ServerPayload,
        description = "The server details",
        content_type = "application/json",
        example = json!({
            "storage": "local",
            "resource": {
                "cpu": 2.5f32,
                "ram": 2048u32,
                "swap": null,
                "disk": null
            },
            "build": {
                "identifier": "minecraft_paper-1.21.1-131-jre21",
                "dockerfile": "<see https://docs.docker.com/get-started/docker-concepts/building-images/writing-a-dockerfile/ for writing a Dockerfile>",
                "build_args": {
                    "JAVA": 21,
                    "MINECRAFT_VERSION": "1.21.1",
                    "PAPER_BUILD": "131"
                },
                "backup_exclude": ["cache", "versions"],
                "files_write": {
                    "eula.txt": {
                        "filetype": "properties",
                        "content": {
                            "eula": "true"
                        }
                    }
                },
                "cap_add": [],
                "cap_drop": ["MKNOD"]
            },
            "ports": [
                {
                    "internal_port": 25565,
                    "external_port": 44955,
                    "external_ip": "0.0.0.0",
                    "protocol": "tcp"
                }
            ]
        })
    ),
    responses(
        (status = 200, description = "Server was created successfully", body = IdResponse, content_type = "application/json"),
        (status = 400, description = "Some parts of the payload are not present"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
     ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn create_server(
    State(state): State<DaemonState>,
    Json(mut payload): Json<ServerPayload>,
) -> Result<Json<IdResponse>, StatusCode> {
    if payload.resource.is_none()
        || payload.build.is_none()
        || payload.ports.is_none()
        || payload.storage.is_none()
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    state
        .config
        .storage
        .get(payload.storage.as_ref().unwrap())
        .ok_or(StatusCode::NOT_FOUND)?;
    let mut conn = state.conn.lock().await;

    let id = conn
        .transaction(move |conn| {
            Box::pin(async move {
                let id = insert_dummy(conn, payload.storage.take().unwrap()).await?;
                modify0(conn, id, &payload).await?;
                Ok::<u32, diesel::result::Error>(id)
            })
        })
        .await
        .map_err(diesel_error_to_http)?;

    Ok(Json(IdResponse { id }))
}

#[utoipa::path(patch,
    path = "/server/{server_id}",
    tag = "server",
    description = "Updates an existing server",
    request_body(
        content = ServerPayload,
        description = "The server details",
        content_type = "application/json",
        example = json!({
            "resource": {
                "cpu": null,
                "ram": 2048u32,
                "swap": 0u32,
                "disk": null
            },
        })
    ),
    responses(
        (status = 200, description = "Server was updated successfully"),
        (status = 404, description = "Server does not exist"),
        (status = 503, description = "Server can't be accessed right now"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 501, description = "You tried to move the server to another storage, which isn't implemened right now"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn modify_server(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Json(payload): Json<ServerPayload>,
) -> Result<(), StatusCode> {
    let mut conn = state.conn.lock().await;
    let _ = server(conn.deref_mut(), &state.servers, server_id)
        .await
        .map_err(diesel_error_to_http)?;

    if payload.storage.is_some() {
        return Err(StatusCode::NOT_IMPLEMENTED);
    }
    conn.transaction(|conn| {
        Box::pin(async move {
            modify0(conn, server_id, &payload).await?;
            Ok::<(), diesel::result::Error>(())
        })
    })
    .await
    .map_err(diesel_error_to_http)?;

    Ok(())
}

#[utoipa::path(delete,
    path = "/server/{server_id}",
    tag = "server",
    description = "Deletes an existing server",
    responses(
        (status = 200, description = "Server was deleted successfully"),
        (status = 404, description = "Server does not exist"),
        (status = 503, description = "Server can't be accessed right now"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn delete_server(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<(), StatusCode> {
    let (server, server_data) = server(
        state.conn.lock().await.deref_mut(),
        &state.servers,
        server_id,
    )
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
                Box::pin(async {
                    delete(schema::servers::dsl::servers)
                        .filter(schema::servers::dsl::id.eq(server_id))
                        .execute(conn)
                        .await?;
                    delete(schema::server_capadd::dsl::server_capadd)
                        .filter(schema::server_capadd::dsl::server_id.eq(server_id))
                        .execute(conn)
                        .await?;
                    delete(schema::server_capdrop::dsl::server_capdrop)
                        .filter(schema::server_capdrop::dsl::server_id.eq(server_id))
                        .execute(conn)
                        .await?;
                    delete(schema::server_buildargs::dsl::server_buildargs)
                        .filter(schema::server_buildargs::dsl::server_id.eq(server_id))
                        .execute(conn)
                        .await?;
                    delete(schema::server_ports::dsl::server_ports)
                        .filter(schema::server_ports::dsl::server_id.eq(server_id))
                        .execute(conn)
                        .await?;

                    Ok(())
                })
            })
            .await
            .map_err(|err: diesel::result::Error| {
                error!("Failed to commit transaction: {:?}", err);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
            state.servers.lock().await.remove(&server_id);

            tokio::task::spawn(async move {
                if let Some(storage) = state.config.storage.get(&server.storage) {
                    let _ = storage::delete(storage, StorageIdentifier::Primary(server_id)).await;
                }
            });

            Ok(())
        }
        Err(_) => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}

// ONLY EXECUTE INSIDE A TRANSACTION
pub(crate) async fn insert_dummy(
    conn: &mut AsyncMysqlConnection,
    storage: String,
) -> Result<u32, diesel::result::Error> {
    insert_into(servers::dsl::servers)
        .values(servers::dsl::storage.eq(storage))
        .execute(conn)
        .await?;
    Ok(select(schema::last_insert_id())
        .get_result::<u64>(conn)
        .await? as u32)
}

// ONLY EXECUTE INSIDE A TRANSACTION
pub(crate) async fn modify0(
    conn: &mut AsyncMysqlConnection,
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
            .execute(conn)
            .await?;
    }
    if let Some(build) = &payload.build {
        update(servers::dsl::servers)
            .filter(filter)
            .set((
                servers::dsl::build_identifier.eq(&build.identifier),
                servers::dsl::build_dockerfile.eq(&build.dockerfile),
            ))
            .execute(conn)
            .await?;
        delete(server_capadd::dsl::server_capadd)
            .filter(server_capadd::dsl::server_id.eq(server_id))
            .execute(conn)
            .await?;
        delete(server_capdrop::dsl::server_capdrop)
            .filter(server_capdrop::dsl::server_id.eq(server_id))
            .execute(conn)
            .await?;
        delete(server_buildargs::dsl::server_buildargs)
            .filter(server_buildargs::dsl::server_id.eq(server_id))
            .execute(conn)
            .await?;
        delete(server_backup_exclude::dsl::server_backup_exclude)
            .filter(server_backup_exclude::dsl::server_id.eq(server_id))
            .execute(conn)
            .await?;
        delete(server_file_writes::dsl::server_file_writes)
            .filter(server_file_writes::dsl::server_id.eq(server_id))
            .execute(conn)
            .await?;

        for (path, file_write) in &build.files_write {
            match file_write {
                ServerFileWrite::WRITE { data } => {
                    insert_into(server_file_writes::dsl::server_file_writes)
                        .values((
                            server_file_writes::dsl::server_id.eq(server_id),
                            server_file_writes::dsl::file.eq(path),
                            server_file_writes::dsl::filetype.eq("write"),
                            server_file_writes::dsl::key.eq(None::<String>),
                            server_file_writes::dsl::value.eq(BASE64_STANDARD.encode(data)),
                        ))
                        .execute(conn)
                        .await?;
                }
                ServerFileWrite::PROPERTIES { content } => {
                    for (key, value) in content {
                        insert_into(server_file_writes::dsl::server_file_writes)
                            .values((
                                server_file_writes::dsl::server_id.eq(server_id),
                                server_file_writes::dsl::file.eq(path),
                                server_file_writes::dsl::filetype.eq("properties"),
                                server_file_writes::dsl::key.eq(Some(key)),
                                server_file_writes::dsl::value.eq(value),
                            ))
                            .execute(conn)
                            .await?;
                    }
                }
            }
        }

        for cap in &build.cap_add {
            insert_into(server_capadd::dsl::server_capadd)
                .values((
                    server_capadd::dsl::server_id.eq(server_id),
                    server_capadd::dsl::capability.eq(cap),
                ))
                .execute(conn)
                .await?;
        }
        for cap in &build.cap_drop {
            insert_into(server_capdrop::dsl::server_capdrop)
                .values((
                    server_capdrop::dsl::server_id.eq(server_id),
                    server_capdrop::dsl::capability.eq(cap),
                ))
                .execute(conn)
                .await?;
        }
        for (key, value) in &build.build_args {
            insert_into(server_buildargs::dsl::server_buildargs)
                .values((
                    server_buildargs::dsl::server_id.eq(server_id),
                    server_buildargs::dsl::key.eq(key),
                    server_buildargs::dsl::value.eq(value),
                ))
                .execute(conn)
                .await?;
        }
        for exclude in &build.backup_exclude {
            insert_into(server_backup_exclude::dsl::server_backup_exclude)
                .values((
                    server_backup_exclude::dsl::server_id.eq(server_id),
                    server_backup_exclude::dsl::exclude.eq(exclude),
                ))
                .execute(conn)
                .await?;
        }
    }
    if let Some(ports) = &payload.ports {
        delete(server_ports::dsl::server_ports)
            .filter(server_ports::dsl::server_id.eq(server_id))
            .execute(conn)
            .await?;
        for port in ports {
            insert_into(server_ports::dsl::server_ports)
                .values((
                    server_ports::dsl::server_id.eq(server_id),
                    server_ports::dsl::internal_port.eq(&port.internal_port),
                    server_ports::dsl::external_port.eq(&port.external_port),
                    server_ports::dsl::external_ip.eq(&port.external_ip),
                    server_ports::dsl::protocol.eq(&port.protocol),
                ))
                .execute(conn)
                .await?;
        }
    }

    Ok(())
}

#[utoipa::path(get,
    path = "/server/{server_id}/start",
    tag = "server",
    description = "Starts an existing server",
    responses(
        (status = 200, description = "Server was started successfully"),
        (status = 404, description = "Server does not exist"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn start_server(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<(), StatusCode> {
    let mut conn = state.conn.clone().lock_owned().await;
    let (server, server_data) = server(&mut conn, &state.servers, server_id)
        .await
        .map_err(diesel_error_to_http)?;

    debug!("Starting Server {:?}", server);

    tokio::task::spawn(async move {
        let (server_data_locked, storage_locked) = tokio::join!(
            timeout(Duration::from_millis(100), server_data.lock()),
            timeout(
                Duration::from_millis(100),
                storage_read(
                    &state.config,
                    &mut conn,
                    &state.storages,
                    StorageIdentifier::Primary(server_id),
                    false
                )
            ),
        );
        if let Ok(mut server_data_locked) = server_data_locked {
            if let Ok(storage) = storage_locked {
                match storage {
                    Ok(storage) => {
                        if let Err(err) = server
                            .start(
                                state.docker.clone(),
                                server_data_locked.deref_mut(),
                                storage,
                                server_data.clone(),
                                state.clone(),
                            )
                            .await
                        {
                            warn!("Failed to start server: {:?}", err);
                        }
                    }
                    Err(err) => {
                        warn!(
                            "Failed to start server: {:?}",
                            ServerStartError::Storage(err)
                        );
                    }
                }
            }
        }
    });

    Ok(())
}

#[utoipa::path(get,
    path = "/server/{server_id}/stop",
    tag = "server",
    description = "Stops an existing server",
    responses(
        (status = 200, description = "Server was stopped successfully"),
        (status = 404, description = "Server does not exist"),
        (status = 503, description = "Server can't be accessed right now"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn stop_server(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<(), StatusCode> {
    let (server, server_data) = server(
        state.conn.lock().await.deref_mut(),
        &state.servers,
        server_id,
    )
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

#[utoipa::path(get,
    path = "/server/{server_id}/kill",
    tag = "server",
    description = "Kills an existing server",
    responses(
        (status = 200, description = "Server was killed successfully"),
        (status = 404, description = "Server does not exist"),
        (status = 503, description = "Server can't be accessed right now"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn kill_server(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<(), StatusCode> {
    let (server, server_data) = server(
        state.conn.lock().await.deref_mut(),
        &state.servers,
        server_id,
    )
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

#[utoipa::path(get,
    path = "/server/{server_id}/restart",
    tag = "server",
    description = "Restarts an existing server",
    responses(
        (status = 200, description = "Server was restarted successfully"),
        (status = 404, description = "Server does not exist"),
        (status = 503, description = "Server can't be accessed right now"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn restart_server(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<(), StatusCode> {
    let (server, server_data) = server(
        state.conn.lock().await.deref_mut(),
        &state.servers,
        server_id,
    )
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

#[utoipa::path(get,
    tag = "server",
    path = "/server/{server_id}",
    description = "Gets the status of an existing server",
    responses(
        (status = 200, description = "Server does exist", body = InternalServerState, content_type = "application/json"),
        (status = 404, description = "Server does not exist"),
        (status = 503, description = "Server can't be accessed right now"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn server_status(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<Json<InternalServerState>, StatusCode> {
    let (server, server_data) = server(
        state.conn.lock().await.deref_mut(),
        &state.servers,
        server_id,
    )
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
