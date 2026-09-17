// SPDX-License-Identifier: AGPL-3.0-only
mod exec;
mod stats;
mod terminal;

use crate::server::FetchedServer;
use crate::server::start::ServerStartError;
use crate::server::status::InternalServerState;
use crate::server::stop::ServerStopError;
use crate::service::server::exec::exec_websocket_handle;
use crate::service::server::stats::stats_websocket_handle;
use crate::service::server::terminal::terminal_websocket_handle;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageError, storage_lock, storage_read_timeout};
use crate::utils::server_required;
use crate::{Daemon, DaemonExtra, sql, storage};
use async_trait::async_trait;
use axum::extract::WebSocketUpgrade;
use base64::Engine as _;
use base64::prelude::BASE64_STANDARD;
use bollard::query_parameters::StatsOptions;
use core::sync::atomic::AtomicU16;
use core::sync::atomic::Ordering;
use core::time::Duration;
use daemon_common::service::files::task::StorageTask;
use daemon_common::service::network::error_network_not_found::FromNetworkNotFound;
use daemon_common::service::network::error_network_suffix_collision::FromNetworkSuffixCollision;
use daemon_common::service::network::error_network_suffix_invalid::FromNetworkSuffixInvalid;
use daemon_common::service::server::error_server_image_different::FromServerImageDifferent;
use daemon_common::service::server::error_server_image_missing_tag::FromServerImageMissingTag;
use daemon_common::service::server::server_service_error::{
    CreateServerError, DeleteServerError, ExecServerError, ExecServerInteractiveError,
    ExecServerInteractiveUserError, KillServerError, RestartServerError, ServerStatsError,
    ServerStatsUserError, ServerStatusError, ServerTerminalError, ServerTerminalUserError,
    StartServerError, StopServerError, UpdateServerError,
};
use daemon_common::service::server::task::ServerTask;
use daemon_common::service::server::{
    ServerBuildImage, ServerBuildPayload, ServerFileWrite, ServerNetworkPayload,
    ServerPortEntryPayload, ServerResourcePayload,
};
use daemon_common::service::server::{ServerService, TasksData};
use daemon_common::service::server::{ServerStats, ServerStatus};
use daemon_common::service::{JsonVec, WebsocketResponse};
use daemon_common::task::TaskDescriptor;
use daemon_common::user::UserSignature;
use futures::TryStreamExt as _;
use sqlx::{Connection as _, MySql, Transaction, query, query_scalar};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::time::timeout;
use tracing::{debug, error, warn};

#[async_trait]
impl<T: DaemonExtra> ServerService for Daemon<T> {
    async fn create_server(
        &self,
        storage: String,
        resource: ServerResourcePayload,
        build: ServerBuildPayload,
        ports: Vec<ServerPortEntryPayload>,
        network: ServerNetworkPayload,
    ) -> Result<u32, CreateServerError> {
        let Some(_) = self.config.storage.get(&storage) else {
            return Err(CreateServerError::StorageLocationNotFound);
        };

        let mut conn = self.conn.acquire().await?;
        conn.transaction(move |tx| {
            Box::pin(async move {
                let id = insert_dummy(tx, storage).await?;
                modify0::<CreateServerError>(
                    tx,
                    id,
                    Some(resource),
                    Some(build),
                    Some(ports),
                    Some(network),
                )
                .await?;
                Ok(id)
            })
        })
        .await
    }

    async fn update_server(
        &self,
        server_id: u32,
        resource: Option<ServerResourcePayload>,
        build: Option<ServerBuildPayload>,
        ports: Option<Vec<ServerPortEntryPayload>>,
        network: Option<ServerNetworkPayload>,
    ) -> Result<(), UpdateServerError> {
        server_required::<UpdateServerError>(&self.conn, &self.servers, server_id).await?;

        let mut conn = self.conn.acquire().await?;
        conn.transaction(|tx| {
            Box::pin(async move {
                modify0::<UpdateServerError>(tx, server_id, resource, build, ports, network).await
            })
        })
        .await
    }

    async fn delete_server(&self, server_id: u32) -> Result<(), DeleteServerError> {
        let (server, server_data) =
            server_required::<DeleteServerError>(&self.conn, &self.servers, server_id).await?;

        let daemon = self.clone();
        match timeout(
            Duration::from_secs(10),
            Arc::clone(&server_data.lock).write_owned(ServerTask::Delete),
        )
        .await
        {
            Ok(lock) => tokio::task::spawn(async move {
                match server.kill(&daemon.docker, &server_data, &lock).await {
                    Ok(()) | Err(ServerStopError::NotRunning) => {}
                    Err(ServerStopError::Docker(err)) => {
                        warn!("Failed to kill server for deletion: {err:?}");
                        return Err(DeleteServerError::DockerFailure);
                    }
                }
                let mut conn = daemon.conn.acquire().await?;

                sql::drop_user(format!("server_{server_id}"), "%")
                    .prepare(&mut *conn)
                    .await?
                    .execute(&mut *conn)
                    .await?;

                query!("DELETE FROM `servers` WHERE `id` = ?", server_id)
                    .execute(&mut *conn)
                    .await?;

                daemon.servers.lock().await.remove(&server_id);

                let mut schedules = daemon.schedules.lock().await;
                if let Some(schedules) = schedules.remove(&server_id) {
                    let mut schedules = schedules.lock().await;
                    for (_, handle) in schedules.schedules.drain() {
                        handle.abort();
                    }
                }
                drop(schedules);

                tokio::task::spawn(async move {
                    if let Some(storage) = daemon.config.storage.get(&server.storage) {
                        let _ =
                            storage::delete(storage, StorageIdentifier::Primary(server_id)).await;
                    }
                    daemon
                        .storages
                        .lock()
                        .await
                        .remove(&StorageIdentifier::Primary(server_id));
                });

                drop(lock);

                Ok(())
            })
            .await
            .expect("Server delete task panicked"),
            Err(_) => {
                debug!("Timeout using Server data lock");
                Err(DeleteServerError::ServerMutexLocked)
            }
        }
    }

    #[expect(clippy::similar_names, reason = "its fine")]
    async fn server_status(&self, server_id: u32) -> Result<ServerStatus, ServerStatusError> {
        let (server, server_data) =
            server_required::<ServerStatusError>(&self.conn, &self.servers, server_id).await?;

        let (_, storage_lock, _, _, storage_value, _, _, _) = storage_lock(
            &self.config,
            &*self.conn,
            &self.storages,
            StorageIdentifier::Primary(server_id),
        )
        .await?
        .ok_or(ServerStatusError::ServerNotFound)?;

        let mut state =
            FetchedServer::container_state(server.id, &server.locks, &self.docker, &server_data)
                .await
                .map_err(|err| {
                    error!("Failed to retrieve server status: {err:?}");
                    ServerStatusError::DockerFailure
                })?;

        let storage_tasks = storage_lock.tasks();
        let server_tasks = server_data.lock.tasks();

        let disk = storage_value.0.load(Ordering::Relaxed);
        let disk_max = storage_value.1.load(Ordering::Relaxed);

        let mut stats = ServerStats::default();

        if matches!(
            state,
            InternalServerState::Running(_, _)
                | InternalServerState::Restarting(_)
                | InternalServerState::Stopping(_)
                | InternalServerState::Unhealthy(_, _)
                | InternalServerState::Starting(_, _)
        ) {
            let mut stream = self.docker.stats(
                &FetchedServer::container_name(server_id),
                Some(StatsOptions {
                    stream: false,
                    one_shot: true,
                }),
            );
            match stream.try_next().await {
                Ok(Some(response)) => stats = ServerStats::from(response),
                Ok(None) => {}
                Err(err) => {
                    warn!("Failed to retrieve container stats: {err:?}");
                }
            }
        }
        stats = stats
            .with_disk(if disk == -1 {
                None
            } else {
                Some(crate::utils::to_mib(disk.unsigned_abs()))
            })
            .with_max(
                server.cpu,
                server.ram,
                server.swap,
                if disk_max == -1 {
                    None
                } else {
                    Some(disk_max.unsigned_abs())
                },
            );

        if server_tasks.iter().any(|desc| {
            matches!(
                desc,
                TaskDescriptor {
                    data: ServerTask::Start,
                    ..
                }
            )
        }) {
            state = InternalServerState::Preparing;
        }

        Ok(ServerStatus {
            state: state.public(),
            tasks: TasksData {
                server: server_tasks,
                storage: storage_tasks,
            },
            stats,
        })
    }

    async fn start_server(&self, server_id: u32) -> Result<(), StartServerError> {
        let (server, server_data) =
            server_required::<StartServerError>(&self.conn, &self.servers, server_id).await?;

        debug!("Starting Server {:?}", server);

        let daemon = self.clone();
        tokio::task::spawn(async move {
            let (lock, storage) = tokio::join!(
                timeout(
                    Duration::from_millis(100),
                    Arc::clone(&server_data.lock).write_owned(ServerTask::Start)
                ),
                storage_read_timeout::<StorageError>(
                    &daemon.config,
                    &*daemon.conn,
                    &daemon.storages,
                    StorageIdentifier::Primary(server_id),
                    StorageTask::ServerRunning,
                )
            );
            if let Ok(mut lock) = lock {
                match storage {
                    Ok(storage) => {
                        if let Err(err) = server
                            .start(
                                Arc::clone(&daemon.docker),
                                server_data,
                                &mut lock,
                                storage,
                                &daemon,
                            )
                            .await
                        {
                            warn!("Failed to start server: {err:?}");
                        }
                    }
                    Err(StorageError::Timeout) => {
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

    async fn stop_server(&self, server_id: u32) -> Result<(), StopServerError> {
        let (server, server_data) =
            server_required::<StopServerError>(&self.conn, &self.servers, server_id).await?;

        let daemon = self.clone();
        tokio::task::spawn(async move {
            let lock = timeout(
                Duration::from_millis(100),
                server_data.lock.read(ServerTask::Stop),
            )
            .await;
            if let Ok(lock) = lock
                && let Err(err) = server.stop(&daemon.docker, &server_data, &lock).await
            {
                warn!("Failed to stop server: {err:?}");
            }
        });

        Ok(())
    }

    async fn kill_server(&self, server_id: u32) -> Result<(), KillServerError> {
        let (server, server_data) =
            server_required::<KillServerError>(&self.conn, &self.servers, server_id).await?;

        let daemon = self.clone();
        tokio::task::spawn(async move {
            let lock = timeout(
                Duration::from_millis(100),
                server_data.lock.read(ServerTask::Kill),
            )
            .await;
            if let Ok(lock) = lock
                && let Err(err) = server.kill(&daemon.docker, &server_data, &lock).await
            {
                warn!("Failed to kill server: {err:?}");
            }
        });

        Ok(())
    }

    async fn restart_server(&self, server_id: u32) -> Result<(), RestartServerError> {
        let (server, server_data) =
            server_required::<RestartServerError>(&self.conn, &self.servers, server_id).await?;

        let state = self.clone();
        tokio::task::spawn(async move {
            let lock = timeout(
                Duration::from_millis(100),
                server_data.lock.read(ServerTask::Restart),
            )
            .await;
            if let Ok(lock) = lock
                && let Err(err) = server.restart(&state.docker, &server_data, &lock).await
            {
                warn!("Failed to restart server: {err:?}");
            }
        });

        Ok(())
    }

    async fn exec_server(&self, server_id: u32, cmd: Vec<String>) -> Result<(), ExecServerError> {
        let (server, server_data) =
            server_required::<ExecServerError>(&self.conn, &self.servers, server_id).await?;

        let state = self.clone();
        tokio::task::spawn(async move {
            let lock = timeout(
                Duration::from_millis(100),
                server_data.lock.read(ServerTask::ContainerCommand),
            )
            .await;
            if let Ok(lock) = lock
                && let Err(err) = server
                    .exec(&state.docker, cmd, &server_data, &lock, false)
                    .await
            {
                warn!("Failed to execute command in server: {err:?}");
            }
        });

        Ok(())
    }

    async fn server_terminal(
        &self,
        ws: WebSocketUpgrade,
        server_id: u32,
        cols: Option<u16>,
        rows: Option<u16>,
        write: bool,
    ) -> Result<WebsocketResponse, ServerTerminalError> {
        let (_, server_data) =
            server_required::<ServerTerminalError>(&self.conn, &self.servers, server_id).await?;
        let docker = Arc::clone(&self.docker);
        Ok(WebsocketResponse(ws.on_upgrade(move |socket| {
            terminal_websocket_handle(
                server_id,
                docker,
                server_data,
                socket,
                Arc::new(AtomicU16::new(cols.unwrap_or_default())),
                Arc::new(AtomicU16::new(rows.unwrap_or_default())),
                write,
            )
        })))
    }

    async fn server_terminal_user(
        &self,
        ws: WebSocketUpgrade,
        server_id: u32,
        cols: Option<u16>,
        rows: Option<u16>,
        write: bool,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, ServerTerminalUserError> {
        verify.verify_terminal::<ServerTerminalUserError>(
            self.root.user_key.as_ref(),
            timestamp,
            &self.key.id,
            server_id,
            write,
        )?;

        let (_, server_data) =
            server_required::<ServerTerminalUserError>(&self.conn, &self.servers, server_id)
                .await?;
        let docker = Arc::clone(&self.docker);
        Ok(WebsocketResponse(ws.on_upgrade(move |socket| {
            terminal_websocket_handle(
                server_id,
                docker,
                server_data,
                socket,
                Arc::new(AtomicU16::new(cols.unwrap_or_default())),
                Arc::new(AtomicU16::new(rows.unwrap_or_default())),
                write,
            )
        })))
    }

    async fn server_stats(
        &self,
        ws: WebSocketUpgrade,
        server_id: u32,
    ) -> Result<WebsocketResponse, ServerStatsError> {
        let (_, server_data) =
            server_required::<ServerStatsError>(&self.conn, &self.servers, server_id).await?;

        let (_, storage_lock, _, _, _, _, _, _) = storage_lock(
            &self.config,
            &*self.conn,
            &self.storages,
            StorageIdentifier::Primary(server_id),
        )
        .await?
        .ok_or(ServerStatsError::ServerNotFound)?;

        let conn = Arc::clone(&self.conn);
        Ok(WebsocketResponse(ws.on_upgrade(move |socket| {
            stats_websocket_handle(server_id, conn, storage_lock, server_data, socket)
        })))
    }

    async fn server_stats_user(
        &self,
        ws: WebSocketUpgrade,
        server_id: u32,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, ServerStatsUserError> {
        verify.verify_stats::<ServerStatsUserError>(
            self.root.user_key.as_ref(),
            timestamp,
            &self.key.id,
            server_id,
        )?;

        let (_, server_data) =
            server_required::<ServerStatsUserError>(&self.conn, &self.servers, server_id).await?;

        let (_, storage_lock, _, _, _, _, _, _) = storage_lock(
            &self.config,
            &*self.conn,
            &self.storages,
            StorageIdentifier::Primary(server_id),
        )
        .await?
        .ok_or(ServerStatsUserError::ServerNotFound)?;

        let conn = Arc::clone(&self.conn);
        Ok(WebsocketResponse(ws.on_upgrade(move |socket| {
            stats_websocket_handle(server_id, conn, storage_lock, server_data, socket)
        })))
    }

    async fn exec_server_interactive(
        &self,
        ws: WebSocketUpgrade,
        server_id: u32,
        cols: Option<u16>,
        rows: Option<u16>,
        command: JsonVec<String>,
    ) -> Result<WebsocketResponse, ExecServerInteractiveError> {
        let (server, server_data) =
            server_required::<ExecServerInteractiveError>(&self.conn, &self.servers, server_id)
                .await?;

        let docker = Arc::clone(&self.docker);
        Ok(WebsocketResponse(ws.on_upgrade(move |socket| {
            exec_websocket_handle(
                server,
                server_data,
                docker,
                command.0,
                socket,
                cols.unwrap_or_default(),
                rows.unwrap_or_default(),
            )
        })))
    }

    async fn exec_server_interactive_user(
        &self,
        ws: WebSocketUpgrade,
        server_id: u32,
        cols: Option<u16>,
        rows: Option<u16>,
        command: JsonVec<String>,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, ExecServerInteractiveUserError> {
        verify.verify_exec::<ExecServerInteractiveUserError>(
            self.root.user_key.as_ref(),
            timestamp,
            &self.key.id,
            server_id,
            &command,
        )?;

        let (server, server_data) =
            server_required::<ExecServerInteractiveUserError>(&self.conn, &self.servers, server_id)
                .await?;

        let docker = Arc::clone(&self.docker);
        Ok(WebsocketResponse(ws.on_upgrade(move |socket| {
            exec_websocket_handle(
                server,
                server_data,
                docker,
                command.0,
                socket,
                cols.unwrap_or_default(),
                rows.unwrap_or_default(),
            )
        })))
    }
}

/// ONLY EXECUTE INSIDE A TRANSACTION
async fn insert_dummy(
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

/// ONLY EXECUTE INSIDE A TRANSACTION
async fn modify0<
    E: FromServerImageDifferent
        + FromServerImageMissingTag
        + FromNetworkNotFound
        + FromNetworkSuffixCollision
        + FromNetworkSuffixInvalid
        + From<sqlx::Error>,
>(
    tx: &mut Transaction<'_, MySql>,
    server_id: u32,
    resource: Option<ServerResourcePayload>,
    build: Option<ServerBuildPayload>,
    ports: Option<Vec<ServerPortEntryPayload>>,
    network: Option<ServerNetworkPayload>,
) -> Result<(), E> {
    if let Some(resource) = &resource {
        query!(
            "UPDATE `servers` SET `cpu` = ?, `ram` = ?, `swap` = ?, `disk` = ? WHERE `id` = ?",
            resource.cpu,
            resource.ram,
            resource.swap,
            resource.disk,
            server_id
        )
        .execute(&mut **tx)
        .await
        .inspect_err(|err| error!("Unable to update resource info of server: {err:?}"))?;
    }
    if let Some(build) = &build {
        query!(
            "DELETE FROM `server_capability_add` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await
        .inspect_err(|err| error!("Unable to delete prior added capabilities: {err:?}"))?;
        query!(
            "DELETE FROM `server_capability_drop` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await
        .inspect_err(|err| error!("Unable to delete prior dropped capabilities: {err:?}"))?;
        query!(
            "DELETE FROM `server_environment` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await
        .inspect_err(|err| {
            error!("Unable to delete prior server environment variables: {err:?}");
        })?;
        query!(
            "DELETE FROM `server_image_tags` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await
        .inspect_err(|err| error!("Unable to delete prior used image tags: {err:?}"))?;
        query!(
            "DELETE FROM `server_image_args` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await
        .inspect_err(|err| error!("Unable to delete prior used image args: {err:?}"))?;

        query!(
            "DELETE FROM `server_backup_exclude` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await
        .inspect_err(|err| error!("Unable to delete prior backup exclusions: {err:?}"))?;
        query!(
            "DELETE FROM `server_file_writes` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await
        .inspect_err(|err| error!("Unable to delete prior set server writes: {err:?}"))?;

        for (path, file_write) in &build.files_write {
            match file_write {
                ServerFileWrite::Write { data } => {
                    query!("INSERT INTO `server_file_writes` (`server`, `file`, `filetype`, `value`) VALUES (?, ?, 'write', ?)", server_id, path, BASE64_STANDARD.encode(data))
                        .execute(&mut **tx)
                        .await
                    .inspect_err(|err| error!("Unable to insert basic server write operation: {err:?}"))?;
                }
                ServerFileWrite::JavaProperties { content } => {
                    for (key, value) in content {
                        query!("INSERT INTO `server_file_writes` (`server`, `file`, `filetype`, `key`, `value`) VALUES (?, ?, 'properties', ?, ?)", server_id, path, key, value)
                            .execute(&mut **tx)
                            .await
                            .inspect_err(|err| error!("Unable to insert properties server write operation: {err:?}"))?;
                    }
                }
                ServerFileWrite::Ini { content } => {
                    for (section, values) in content {
                        for (key, value) in values {
                            query!("INSERT INTO `server_file_writes` (`server`, `file`, `filetype`, `key`, `value`) VALUES (?, ?, 'ini', ?, ?)", server_id, path, serde_json::to_string(&(section, key)).expect("Unable to serialize ini key"), value)
                                .execute(&mut **tx)
                                .await
                                .inspect_err(|err| error!("Unable to insert ini server write operation: {err:?}"))?;
                        }
                    }
                }
                ServerFileWrite::Json { set } => {
                    for patch in set {
                        query!("INSERT INTO `server_file_writes` (`server`, `file`, `filetype`, `value`) VALUES (?, ?, 'json', ?)", server_id, path, serde_json::to_string(patch).expect("Unable to serialize patch"))
                            .execute(&mut **tx)
                            .await
                            .inspect_err(|err| error!("Unable to insert json server write operation: {err:?}"))?;
                    }
                }
                ServerFileWrite::Yaml { set } => {
                    for patch in set {
                        query!("INSERT INTO `server_file_writes` (`server`, `file`, `filetype`, `value`) VALUES (?, ?, 'yaml', ?)", server_id, path, serde_json::to_string(patch).expect("Unable to serialize patch"))
                            .execute(&mut **tx)
                            .await
                            .inspect_err(|err| error!("Unable to insert yaml server write operation: {err:?}"))?;
                    }
                }
                ServerFileWrite::Toml { set } => {
                    query!("INSERT INTO `server_file_writes` (`server`, `file`, `filetype`, `value`) VALUES (?, ?, 'toml', ?)", server_id, path, serde_json::to_string(set).expect("Unable to serialize toml patches"))
                        .execute(&mut **tx)
                        .await
                        .inspect_err(|err| error!("Unable to insert toml server write operation: {err:?}"))?;
                }
                ServerFileWrite::Xml { set } => {
                    query!("INSERT INTO `server_file_writes` (`server`, `file`, `filetype`, `value`) VALUES (?, ?, 'xml', ?)", server_id, path, serde_json::to_string(set).expect("Unable to serialize xml patches"))
                        .execute(&mut **tx)
                        .await
                        .inspect_err(|err| error!("Unable to insert xml server write operation: {err:?}"))?;
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
            .await
            .inspect_err(|err| error!("Unable to insert added capability: {err:?}"))?;
        }
        for cap in &build.cap_drop {
            query!(
                "INSERT INTO `server_capability_drop` (`server`, `capability`) VALUES (?, ?)",
                server_id,
                cap
            )
            .execute(&mut **tx)
            .await
            .inspect_err(|err| error!("Unable to insert dropped capability: {err:?}"))?;
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
                return Err(<E as FromServerImageMissingTag>::from());
            }

            if let Some(default_tag) = query_scalar!(
                "SELECT `default_tag` FROM `images` WHERE `id` = ?",
                image_id
            )
            .fetch_optional(&mut **tx)
            .await?
            {
                if !default_tag.eq(&image.default_tag) {
                    return Err(<E as FromServerImageDifferent>::from());
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
                    return Err(<E as FromServerImageDifferent>::from());
                }
            }
            query!(
                "INSERT IGNORE INTO `images` (`id`, `dockerfile`, `default_tag`) VALUES (?, ?, ?)",
                image_id,
                image.dockerfile(),
                &image.default_tag
            )
            .execute(&mut **tx)
            .await
            .inspect_err(|err| error!("Unable to insert required server images: {err:?}"))?;
            if let Some(tag) = &image.tag {
                query!("INSERT IGNORE INTO `server_image_tags` (`server`, `image`, `tag`) VALUES (?, ?, ?)", server_id, image_id, tag)
                    .execute(&mut **tx)
                    .await
                    .inspect_err(|err| error!("Unable to insert server image tags: {err:?}"))?;
            }
            for (key, value) in &image.args {
                query!("INSERT IGNORE INTO `server_image_args` (`server`, `image`, `key`, `value`) VALUES (?, ?, ?, ?)", server_id, image_id, key, value)
                    .execute(&mut **tx)
                    .await
                    .inspect_err(|err| error!("Unable to insert server image args: {err:?}"))?;
            }

            for parent in parents {
                query!(
                    "INSERT IGNORE INTO `image_parents` (`image`, `parent`) VALUES (?, ?)",
                    image_id,
                    parent
                )
                .execute(&mut **tx)
                .await
                .inspect_err(|err| error!("Unable to insert server image parents: {err:?}"))?;
            }
        }
        for exclude in &build.backup_exclude {
            query!(
                "INSERT INTO `server_backup_exclude` (`server`, `exclude`) VALUES (?, ?)",
                server_id,
                exclude
            )
            .execute(&mut **tx)
            .await
            .inspect_err(|err| error!("Unable to insert server backup exclusion: {err:?}"))?;
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
        .await
        .inspect_err(|err| error!("Unable to update server build info: {err:?}"))?;
    }
    if let Some(ports) = &ports {
        query!("DELETE FROM `server_ports` WHERE `server` = ?", server_id)
            .execute(&mut **tx)
            .await
            .inspect_err(|err| error!("Unable to remove prior server ports: {err:?}"))?;
        for port in ports {
            query!("INSERT INTO `server_ports` (`server`, `internal_port`, `external_port`, `external_ip`, `protocol`) VALUES (?, ?, ?, ?, ?)", server_id, &port.internal_port, &port.external_port, &port.external_ip, &port.protocol)
                .execute(&mut **tx)
                .await
                .inspect_err(|err| error!("Unable to insert server port: {err:?}"))?;
        }
    }
    if let Some(network) = &network {
        query!(
            "UPDATE `servers` SET `internet` = ? WHERE `id` = ?",
            network.internet_access,
            server_id
        )
        .execute(&mut **tx)
        .await
        .inspect_err(|err| error!("Unable to update server network info: {err:?}"))?;
        query!(
            "DELETE FROM `server_networks` WHERE `server` = ?",
            server_id
        )
        .execute(&mut **tx)
        .await
        .inspect_err(|err| error!("Unable to remove prior server network attachments: {err:?}"))?;
        for attachment in &network.networks {
            let ipv4_network = query_scalar!(
                "SELECT `ipv4` IS NOT NULL AS `ipv4: bool` FROM `networks` WHERE `id` = ?",
                attachment.network
            )
            .fetch_optional(&mut **tx)
            .await
            .inspect_err(|err| error!("Unable to fetch network: {err:?}"))?
            .ok_or_else(<E as FromNetworkNotFound>::from)?;

            if ipv4_network != attachment.ipv4.is_some() {
                return Err(<E as FromNetworkSuffixInvalid>::from());
            }

            query!("INSERT INTO `server_networks` (`server`, `network`, `ipv4`, `ipv6`) VALUES (?, ?, ?, ?)", server_id, attachment.network, attachment.ipv4, attachment.ipv6)
                .execute(&mut **tx)
                .await
                .map_err(|err| {
               		if let sqlx::Error::Database(ref db) = err {
                		if db.is_unique_violation() {
                 			return <E as FromNetworkSuffixCollision>::from();
                 		}
                  		if db.is_check_violation() {
                   			return <E as FromNetworkSuffixInvalid>::from();
                    	}
                 	}

                	error!("Unable to insert server network attachments: {err:?}");

                 	<E as From<sqlx::Error>>::from(err)
                })?;
        }
    }

    Ok(())
}
