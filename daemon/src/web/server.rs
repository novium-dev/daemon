use std::collections::HashSet;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::common::payload::server::ServerPayload;
use crate::common::payload::IdResponse;
use crate::server::start::ServerStartError;
use crate::server::status::InternalServerState;
use crate::server::stop::ServerStopError;
use crate::server::FetchedServer;
use crate::storage::sync::{storage_read, storage_write, StorageError, StorageLockData};
use crate::storage::StorageIdentifier;
use crate::utils::server;
use crate::web::sql_error_to_http;
use crate::{sql, storage, DaemonState};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use daemon_common::payload::server::{ServerBuildImage, ServerFileWrite};
use sqlx::{query, query_scalar, Connection, Error, MySql, Transaction};
use tokio::time::timeout;
use tracing::{debug, error, warn};
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
        .routes(routes!(exec_server))
        .routes(routes!(wipe_server))
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
                "image": {
                    "tag": "minecraft_paper-1.21.1-131-jre21",
                    "default_tag": null,
                    "dockerfile": "<see https://docs.docker.com/get-started/docker-concepts/building-images/writing-a-dockerfile/ for writing a Dockerfile>",
                    "args": {
                        "JAVA": 21,
                        "MINECRAFT_VERSION": "1.21.1",
                        "PAPER_BUILD": "131"
                    },
                    "parents": [],
                    "base": null,
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
        (status = 400, description = "Some parts of the payload are not present or the network payload is invalid"),
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
        || payload.network.is_none()
    {
        debug!("Some parts of the server create payload are missing");
        return Err(StatusCode::BAD_REQUEST);
    }
    state
        .config
        .storage
        .get(payload.storage.as_ref().unwrap())
        .ok_or(StatusCode::NOT_FOUND)
        .inspect_err(|_err| debug!("Server storage does not exist: {:?}", payload.storage))?;

    let id = {
        let mut conn = state.conn.acquire().await.map_err(sql_error_to_http)?;
        conn.transaction(move |tx| {
            Box::pin(async move {
                let id = insert_dummy(tx, payload.storage.take().unwrap()).await?;
                modify0(tx, id, &payload).await?;
                Ok::<_, ServerModifyError>(id)
            })
        })
        .await
        .map_err(|err| match err {
            ServerModifyError::Sql(sql) => sql_error_to_http(sql),
            ServerModifyError::DifferentImage => StatusCode::BAD_REQUEST,
            ServerModifyError::MissingImageTag => StatusCode::BAD_REQUEST,
        })?
    };
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
        (status = 401, description = "Your authorization token is invalid"),
        (status = 400, description = "The network payload is invalid")
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
    let Some(_) = server(state.conn.deref(), &state.servers, server_id)
        .await
        .map_err(sql_error_to_http)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };

    if payload.storage.is_some() {
        debug!("Tried to move the server");
        return Err(StatusCode::NOT_IMPLEMENTED);
    }
    {
        let mut conn = state.conn.acquire().await.map_err(sql_error_to_http)?;
        conn.transaction(|tx| Box::pin(async move { modify0(tx, server_id, &payload).await }))
            .await
            .map_err(|err| match err {
                ServerModifyError::Sql(sql) => sql_error_to_http(sql),
                ServerModifyError::DifferentImage => StatusCode::BAD_REQUEST,
                ServerModifyError::MissingImageTag => StatusCode::BAD_REQUEST,
            })?;
    }

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
    let Some((server, server_data)) = server(state.conn.deref(), &state.servers, server_id)
        .await
        .map_err(sql_error_to_http)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    match timeout(Duration::from_secs(10), server_data.lock_owned()).await {
        Ok(server_data_locked) => {
            match server.kill(&state.docker, server_data_locked).await {
                Ok(_) | Err(ServerStopError::NotRunning) => {}
                Err(err) => {
                    warn!("Failed to kill server for deletion: {:?}", err);
                    return Err(StatusCode::INTERNAL_SERVER_ERROR);
                }
            }
            let mut conn = state.conn.acquire().await.map_err(sql_error_to_http)?;

            sql::drop_user(format!("server_{}", server_id), "%")
                .prepare(&mut *conn)
                .await
                .map_err(sql_error_to_http)?
                .execute(&mut *conn)
                .await
                .map_err(sql_error_to_http)?;

            query!("DELETE FROM `servers` WHERE `id` = ?", server_id)
                .execute(&mut *conn)
                .await
                .map_err(sql_error_to_http)?;

            state.servers.lock().await.remove(&server_id);

            tokio::task::spawn(async move {
                if let Some(storage) = state.config.storage.get(&server.storage) {
                    let _ = storage::delete(storage, StorageIdentifier::Primary(server_id)).await;
                }
                state
                    .storages
                    .lock()
                    .await
                    .remove(&StorageIdentifier::Primary(server_id));
            });

            Ok(())
        }
        Err(_) => {
            debug!("Timeout using Server data lock");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
    }
}

// ONLY EXECUTE INSIDE A TRANSACTION
pub(crate) async fn insert_dummy(
    conn: &mut Transaction<'_, MySql>,
    storage: String,
) -> Result<u32, sqlx::Error> {
    Ok(
        query!("INSERT INTO `servers` (`storage`) VALUES (?)", storage)
            .execute(&mut **conn)
            .await?
            .last_insert_id() as u32,
    )
}

pub(crate) enum ServerModifyError {
    Sql(sqlx::Error),
    DifferentImage,
    MissingImageTag,
}

impl From<sqlx::Error> for ServerModifyError {
    fn from(value: Error) -> Self {
        Self::Sql(value)
    }
}

// ONLY EXECUTE INSIDE A TRANSACTION
async fn modify0(
    tx: &mut Transaction<'_, MySql>,
    server_id: u32,
    payload: &ServerPayload,
) -> Result<(), ServerModifyError> {
    if let Some(resource) = &payload.resource {
        query!(
            "UPDATE `servers` SET `cpu` = ?, `ram` = ?, `swap` = ?, `disk` = ? WHERE `id` = ?",
            resource.cpu,
            resource.ram,
            resource.swap,
            resource.disk,
            server_id
        )
        .execute(&mut **tx)
        .await?;
    }
    if let Some(build) = &payload.build {
        query!(
            "DELETE FROM `server_capability_add` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await?;
        query!(
            "DELETE FROM `server_capability_drop` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await?;
        query!(
            "DELETE FROM `server_environment` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await?;
        query!(
            "DELETE FROM `server_image_tags` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await?;
        query!(
            "DELETE FROM `server_image_args` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await?;

        query!(
            "DELETE FROM `server_backup_exclude` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await?;
        query!(
            "DELETE FROM `server_file_writes` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await?;

        for (path, file_write) in &build.files_write {
            match file_write {
                ServerFileWrite::WRITE { data } => {
                    query!("INSERT INTO `server_file_writes` (`server`, `file`, `filetype`, `key`, `value`) VALUES (?, ?, 'write', NULL, ?)", server_id, path, BASE64_STANDARD.encode(data))
                        .execute(&mut **tx)
                        .await?;
                }
                ServerFileWrite::PROPERTIES { content } => {
                    for (key, value) in content {
                        query!("INSERT INTO `server_file_writes` (`server`, `file`, `filetype`, `key`, `value`) VALUES (?, ?, 'properties', ?, ?)", server_id, path, key, value)
                            .execute(&mut **tx)
                            .await?;
                    }
                }
                ServerFileWrite::JSON { content } => {
                    for (key, value) in content {
                        query!("INSERT INTO `server_file_writes` (`server`, `file`, `filetype`, `key`, `value`) VALUES (?, ?, 'json', ?, ?)", server_id, path, key, serde_json::to_string(value).unwrap())
                            .execute(&mut **tx)
                            .await?;
                    }
                }
                ServerFileWrite::YAML { content } => {
                    for (key, value) in content {
                        query!("INSERT INTO `server_file_writes` (`server`, `file`, `filetype`, `key`, `value`) VALUES (?, ?, 'yaml', ?, ?)", server_id, path, key, serde_yaml::to_string(value).unwrap())
                            .execute(&mut **tx)
                            .await?;
                    }
                }
            }
        }

        for cap in &build.cap_add {
            query!(
                "INSERT INTO `server_capability_add` (`server`, `capability`) VALUES (?, ?)",
                server_id,
                cap
            )
            .execute(&mut **tx)
            .await?;
        }
        for cap in &build.cap_drop {
            query!(
                "INSERT INTO `server_capability_drop` (`server`, `capability`) VALUES (?, ?)",
                server_id,
                cap
            )
            .execute(&mut **tx)
            .await?;
        }
        for (key, val) in &build.environment {
            query!(
                "INSERT INTO `server_environment` (`server`, `key`, `value`) VALUES (?, ?, ?)",
                server_id,
                key,
                val,
            )
            .execute(&mut **tx)
            .await?;
        }
        for image in build.image.all() {
            let image_id = image.id();
            let parents: HashSet<u64> = image.parents.iter().map(ServerBuildImage::id).collect();

            if image.default_tag.is_none() && image.tag.is_none() {
                return Err(ServerModifyError::MissingImageTag);
            }

            if let Some(default_tag) = query_scalar!(
                "SELECT `default_tag` FROM `images` WHERE `id` = ?",
                image_id
            )
            .fetch_optional(&mut **tx)
            .await?
            {
                if !default_tag.eq(&image.default_tag) {
                    return Err(ServerModifyError::DifferentImage);
                }

                let parents_current = query_scalar!(
                    "SELECT `parent` FROM `image_parents` WHERE `image` = ?",
                    image_id
                )
                .fetch_all(&mut **tx)
                .await?
                .into_iter()
                .collect();
                if !parents.eq(&parents_current) {
                    return Err(ServerModifyError::DifferentImage);
                }
            }
            query!(
                "INSERT IGNORE INTO `images` (`id`, `dockerfile`, `default_tag`) VALUES (?, ?, ?)",
                image_id,
                image.dockerfile(),
                &image.default_tag
            )
            .execute(&mut **tx)
            .await?;
            if let Some(tag) = &image.tag {
                query!("INSERT IGNORE INTO `server_image_tags` (`server`, `image`, `tag`) VALUES (?, ?, ?)", server_id, image_id, tag)
                    .execute(&mut **tx)
                    .await?;
            }
            for (key, value) in &image.args {
                query!("INSERT IGNORE INTO `server_image_args` (`server`, `image`, `key`, `value`) VALUES (?, ?, ?, ?)", server_id, image_id, key, value)
                    .execute(&mut **tx)
                    .await?;
            }

            for parent in parents {
                query!(
                    "INSERT IGNORE INTO `image_parents` (`image`, `parent`) VALUES (?, ?)",
                    image_id,
                    parent
                )
                .execute(&mut **tx)
                .await?;
            }
        }
        for exclude in &build.backup_exclude {
            query!(
                "INSERT INTO `server_backup_exclude` (`server`, `exclude`) VALUES (?, ?)",
                server_id,
                exclude
            )
            .execute(&mut **tx)
            .await?;
        }

        query!(
            "UPDATE `servers` SET `uid` = ?, `gid` = ?, `image` = ?, `stop_command` = ?, `stop_timeout` = ? WHERE `id` = ?",
            build.uid,
            build.gid,
            build.image.id(),
            build.stop_command,
            build.stop_timeout,
            server_id,
        )
        .execute(&mut **tx)
        .await?;
    }
    if let Some(ports) = &payload.ports {
        query!("DELETE FROM `server_ports` WHERE `server` = ?", server_id)
            .execute(&mut **tx)
            .await?;
        for port in ports {
            query!("INSERT INTO `server_ports` (`server`, `internal_port`, `external_port`, `external_ip`, `protocol`) VALUES (?, ?, ?, ?, ?)", server_id, &port.internal_port, &port.external_port, &port.external_ip, &port.protocol)
                .execute(&mut **tx)
                .await?;
        }
    }
    if let Some(network) = &payload.network {
        query!(
            "UPDATE `servers` SET `internet` = ? WHERE `id` = ?",
            network.internet_access,
            server_id
        )
        .execute(&mut **tx)
        .await?;
        query!(
            "DELETE FROM `server_networks` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await?;
        for attachment in &network.networks {
            query!("INSERT INTO `server_networks` (`server`, `network`, `ipv4`, `ipv6`) VALUES (?, ?, ?, ?)", server_id, attachment.network, attachment.ipv4, attachment.ipv6)
                .execute(&mut **tx)
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
    let Some((server, server_data)) = server(state.conn.deref(), &state.servers, server_id)
        .await
        .map_err(sql_error_to_http)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };

    debug!("Starting Server {:?}", server);

    tokio::task::spawn(async move {
        let (server_data_locked, storage_locked) = tokio::join!(
            timeout(Duration::from_millis(100), server_data.lock()),
            timeout(
                Duration::from_millis(100),
                storage_read(
                    &state.config,
                    state.conn.deref(),
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
                                &state,
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
            } else {
                debug!("Timeout locking server data. Ignoring start request...");
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
    let Some((server, server_data)) = server(state.conn.deref(), &state.servers, server_id)
        .await
        .map_err(sql_error_to_http)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };

    tokio::task::spawn(async move {
        let server_data = timeout(Duration::from_millis(100), server_data.lock_owned()).await;
        if let Ok(server_data) = server_data {
            if let Err(err) = server.stop(&state.docker, server_data).await {
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
    let Some((server, server_data)) = server(state.conn.deref(), &state.servers, server_id)
        .await
        .map_err(sql_error_to_http)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };

    tokio::task::spawn(async move {
        let server_data = timeout(Duration::from_millis(100), server_data.lock_owned()).await;
        if let Ok(server_data) = server_data {
            if let Err(err) = server.kill(&state.docker, server_data).await {
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
    let Some((server, server_data)) = server(state.conn.deref(), &state.servers, server_id)
        .await
        .map_err(sql_error_to_http)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };

    tokio::task::spawn(async move {
        let server_data = timeout(Duration::from_millis(100), server_data.lock_owned()).await;
        if let Ok(server_data) = server_data {
            if let Err(err) = server.restart(&state.docker, server_data).await {
                warn!("Failed to restart server: {:?}", err);
            }
        }
    });

    Ok(())
}

#[utoipa::path(post,
    path = "/server/{server_id}/exec",
    tag = "server",
    description = "Executes a command in an existing server",
    request_body(
        content = Vec<String>,
        description = "The command",
        content_type = "application/json",
        example = json!(["/usr/bin/echo", "test"])
    ),
    responses(
        (status = 200, description = "Command in Server was executed successfully"),
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
pub(crate) async fn exec_server(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
    Json(cmd): Json<Vec<String>>,
) -> Result<(), StatusCode> {
    let Some((server, server_data)) = server(state.conn.deref(), &state.servers, server_id)
        .await
        .map_err(sql_error_to_http)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };

    tokio::task::spawn(async move {
        let server_data = timeout(Duration::from_millis(100), server_data.lock()).await;
        if let Ok(mut server_data) = server_data {
            if let Err(err) = server
                .exec(&state.docker, cmd, server_data.deref_mut(), false)
                .await
            {
                warn!("Failed to execute command in server: {:?}", err);
            }
        }
    });

    Ok(())
}

#[utoipa::path(get,
    path = "/server/{server_id}/wipe",
    tag = "server",
    description = "Deletes all files in an existing server",
    responses(
        (status = 200, description = "Command in Server was executed successfully"),
        (status = 404, description = "Server does not exist"),
        (status = 503, description = "Storage can't be accessed right now"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn wipe_server(
    State(state): State<DaemonState>,
    Path((server_id,)): Path<(u32,)>,
) -> Result<(), StatusCode> {
    let StorageLockData {
        lock,
        storage: entry,
        value: storage_value,
        ..
    } = storage_write(
        &state.config,
        state.conn.deref(),
        &state.storages,
        StorageIdentifier::Primary(server_id),
        true,
    )
    .await
    .map_err(StorageError::into_http)?;
    storage::delete(entry, StorageIdentifier::Primary(server_id))
        .await
        .inspect_err(|err| error!("Failed to wipe server: {:?}", err))
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    storage_value.0.store(0, Ordering::Relaxed);

    drop(lock);
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
    let Some((server, server_data)) = server(state.conn.deref(), &state.servers, server_id)
        .await
        .map_err(sql_error_to_http)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    let server_data = timeout(Duration::from_millis(100), server_data.lock()).await;
    match server_data {
        Ok(mut server_data) => Ok(Json(
            FetchedServer::state(
                server.id,
                server.locks,
                &state.docker,
                server_data.deref_mut(),
            )
            .await
            .inspect_err(|err| debug!("Failed to get current server state: {:?}", err))
            .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?,
        )),
        Err(_err) => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}
