use crate::server::start::ServerStartError;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageError, StorageLockData, storage_read, storage_write};
use crate::utils::server;
use crate::web::sql_error_to_http;
use crate::{Daemon, storage};
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, error, warn};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub(crate) fn config() -> OpenApiRouter<Daemon> {
    OpenApiRouter::new()
        .routes(routes!(start_server))
        .routes(routes!(stop_server))
        .routes(routes!(kill_server))
        .routes(routes!(restart_server))
        .routes(routes!(exec_server))
        .routes(routes!(wipe_server))
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
    State(state): State<Daemon>,
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
        let (server_data_locked, storage) = tokio::join!(
            timeout(Duration::from_millis(100), server_data.lock()),
            storage_read(
                &state.config,
                state.conn.deref(),
                &state.storages,
                StorageIdentifier::Primary(server_id),
                true
            )
        );
        if let Ok(mut server_data_locked) = server_data_locked {
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
                Err(StorageError::Timeout(_)) => {
                    debug!("Timeout locking storage. Ignoring start request...");
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
    State(state): State<Daemon>,
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
    State(state): State<Daemon>,
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
    State(state): State<Daemon>,
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
    State(state): State<Daemon>,
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
    State(state): State<Daemon>,
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
    storage::delete(&entry, StorageIdentifier::Primary(server_id))
        .await
        .inspect_err(|err| error!("Failed to wipe server: {:?}", err))
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    storage_value.0.store(0, Ordering::Relaxed);

    drop(lock);
    Ok(())
}

