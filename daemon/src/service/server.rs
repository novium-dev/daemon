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
mod exec;
mod stats;
mod terminal;

use crate::server::FetchedServer;
use crate::server::start::ServerStartError;
use crate::server::stop::ServerStopError;
use crate::service::server::exec::exec_websocket_handle;
use crate::service::server::stats::stats_websocket_handle;
use crate::service::server::terminal::terminal_websocket_handle;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{
    StorageError, StorageLockData, storage_read_timeout, storage_write_timeout,
};
use crate::utils::server_required;
use crate::{Daemon, DaemonExtra, sql, storage};
use async_trait::async_trait;
use axum::extract::WebSocketUpgrade;
use base64::Engine as _;
use base64::prelude::BASE64_STANDARD;
#[cfg(not(feature = "no_tty_size_please"))]
use core::sync::atomic::AtomicU16;
use core::sync::atomic::Ordering;
use core::time::Duration;
use daemon_common::service::server::PublicServerState;
use daemon_common::service::server::ServerService;
use daemon_common::service::server::error::{
    CreateServerError, DeleteServerError, ExecServerError, ExecServerInteractiveError,
    KillServerError, RestartServerError, ServerStatsError, ServerStatusError, ServerTerminalError,
    StartServerError, StopServerError, UpdateServerError, WipeServerError,
};
use daemon_common::service::server::error_server_image_different::FromServerImageDifferent;
use daemon_common::service::server::error_server_image_missing_tag::FromServerImageMissingTag;
use daemon_common::service::server::{
    ServerBuildImage, ServerBuildPayload, ServerFileWrite, ServerNetworkPayload,
    ServerPortEntryPayload, ServerResourcePayload,
};
use daemon_common::service::{JsonVec, WebsocketResponse};
use daemon_common::user::UserSignature;
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
        match timeout(Duration::from_secs(10), server_data.lock_owned()).await {
            Ok(server_data_locked) => tokio::task::spawn(async move {
                match server.kill(&daemon.docker, server_data_locked).await {
                    Ok(()) | Err(ServerStopError::NotRunning) => {}
                    Err(ServerStopError::Docker(docker)) => {
                        warn!("Failed to kill server for deletion: {:?}", docker);
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

    async fn server_status(
        &self,
        server_id: u32,
        precise: Option<bool>,
    ) -> Result<PublicServerState, ServerStatusError> {
        let (server, server_data) =
            server_required::<ServerStatusError>(&self.conn, &self.servers, server_id).await?;

        #[expect(
            clippy::if_then_some_else_none,
            reason = "await is used, there is no then_async yet"
        )]
        let server_data = if precise.unwrap_or(true) {
            let server_data = timeout(Duration::from_millis(100), server_data.lock()).await;
            Some(server_data.map_err(|_elapsed| ServerStatusError::ServerMutexLocked)?)
        } else {
            None
        };
        match FetchedServer::state(
            server.id,
            server.locks,
            &self.docker,
            server_data.as_deref(),
        )
        .await
        {
            Ok(state) => Ok(state.public()),
            Err(docker) => {
                error!("Failed to retrieve server status: {:?}", docker);
                Err(ServerStatusError::DockerFailure)
            }
        }
    }

    async fn start_server(&self, server_id: u32) -> Result<(), StartServerError> {
        let (server, server_data) =
            server_required::<StartServerError>(&self.conn, &self.servers, server_id).await?;

        debug!("Starting Server {:?}", server);

        let daemon = self.clone();
        tokio::task::spawn(async move {
            let (server_data_locked, storage) = tokio::join!(
                timeout(Duration::from_millis(100), server_data.lock()),
                storage_read_timeout::<StorageError>(
                    &daemon.config,
                    &*daemon.conn,
                    &daemon.storages,
                    StorageIdentifier::Primary(server_id),
                )
            );
            if let Ok(mut server_data_locked) = server_data_locked {
                match storage {
                    Ok(storage) => {
                        if let Err(err) = server
                            .start(
                                Arc::clone(&daemon.docker),
                                &mut server_data_locked,
                                storage,
                                Arc::clone(&server_data),
                                &daemon,
                            )
                            .await
                        {
                            warn!("Failed to start server: {:?}", err);
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
            let server_data = timeout(Duration::from_millis(100), server_data.lock_owned()).await;
            if let Ok(server_data) = server_data {
                if let Err(err) = server.stop(&daemon.docker, server_data).await {
                    warn!("Failed to stop server: {:?}", err);
                }
            }
        });

        Ok(())
    }

    async fn kill_server(&self, server_id: u32) -> Result<(), KillServerError> {
        let (server, server_data) =
            server_required::<KillServerError>(&self.conn, &self.servers, server_id).await?;

        let daemon = self.clone();
        tokio::task::spawn(async move {
            let server_data = timeout(Duration::from_millis(100), server_data.lock_owned()).await;
            if let Ok(server_data) = server_data {
                if let Err(err) = server.kill(&daemon.docker, server_data).await {
                    warn!("Failed to kill server: {:?}", err);
                }
            }
        });

        Ok(())
    }

    async fn restart_server(&self, server_id: u32) -> Result<(), RestartServerError> {
        let (server, server_data) =
            server_required::<RestartServerError>(&self.conn, &self.servers, server_id).await?;

        let state = self.clone();
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

    async fn exec_server(&self, server_id: u32, cmd: Vec<String>) -> Result<(), ExecServerError> {
        let (server, server_data) =
            server_required::<ExecServerError>(&self.conn, &self.servers, server_id).await?;

        let state = self.clone();
        tokio::task::spawn(async move {
            let server_data = timeout(Duration::from_millis(100), server_data.lock()).await;
            if let Ok(server_data) = server_data {
                if let Err(err) = server.exec(&state.docker, cmd, &server_data, false).await {
                    warn!("Failed to execute command in server: {:?}", err);
                }
            }
        });

        Ok(())
    }

    async fn wipe_server(&self, server_id: u32) -> Result<(), WipeServerError> {
        let StorageLockData {
            lock,
            storage: entry,
            value: storage_value,
            ..
        } = storage_write_timeout::<WipeServerError>(
            &self.config,
            &*self.conn,
            &self.storages,
            StorageIdentifier::Primary(server_id),
        )
        .await?;
        storage::delete(&entry, StorageIdentifier::Primary(server_id))
            .await
            .inspect_err(|err| error!("Failed to wipe server: {:?}", err))?;
        storage_value.0.store(0, Ordering::Relaxed);

        drop(lock);
        Ok(())
    }

    async fn server_terminal(
        &self,
        ws: WebSocketUpgrade,
        server_id: u32,
        cols: Option<u16>,
        rows: Option<u16>,
        write: bool,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, ServerTerminalError> {
        #[cfg(feature = "no_tty_size_please")]
        let _ = (cols, rows);

        verify.verify_terminal::<ServerTerminalError>(
            self.root.user_key.as_ref(),
            timestamp,
            &self.key.id,
            server_id,
            write,
        )?;

        let (_, server_data) =
            server_required::<ServerTerminalError>(&self.conn, &self.servers, server_id).await?;
        #[cfg(not(feature = "no_tty_size_please"))]
        let docker = Arc::clone(&self.docker);
        Ok(WebsocketResponse(ws.on_upgrade(move |socket| {
            terminal_websocket_handle(
                server_id,
                #[cfg(not(feature = "no_tty_size_please"))]
                docker,
                server_data,
                socket,
                #[cfg(not(feature = "no_tty_size_please"))]
                Arc::new(AtomicU16::new(cols.unwrap_or_default())),
                #[cfg(not(feature = "no_tty_size_please"))]
                Arc::new(AtomicU16::new(rows.unwrap_or_default())),
                write,
            )
        })))
    }

    async fn server_stats(
        &self,
        ws: WebSocketUpgrade,
        server_id: u32,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, ServerStatsError> {
        verify.verify_stats::<ServerStatsError>(
            self.root.user_key.as_ref(),
            timestamp,
            &self.key.id,
            server_id,
        )?;

        let (_, server_data) =
            server_required::<ServerStatsError>(&self.conn, &self.servers, server_id).await?;
        let conn = Arc::clone(&self.conn);
        Ok(WebsocketResponse(ws.on_upgrade(move |socket| {
            stats_websocket_handle(server_id, conn, server_data, socket)
        })))
    }

    async fn exec_server_interactive(
        &self,
        ws: WebSocketUpgrade,
        server_id: u32,
        cols: Option<u16>,
        rows: Option<u16>,
        command: JsonVec<String>,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, ExecServerInteractiveError> {
        verify.verify_exec::<ExecServerInteractiveError>(
            self.root.user_key.as_ref(),
            timestamp,
            &self.key.id,
            server_id,
            &command,
        )?;

        let (server, server_data) =
            server_required::<ExecServerInteractiveError>(&self.conn, &self.servers, server_id)
                .await?;
        #[cfg(feature = "no_tty_size_please")]
        let _ = (cols, rows);

        let docker = Arc::clone(&self.docker);
        Ok(WebsocketResponse(ws.on_upgrade(move |socket| {
            exec_websocket_handle(
                server,
                server_data,
                docker,
                command.0,
                socket,
                #[cfg(not(feature = "no_tty_size_please"))]
                cols.unwrap_or_default(),
                #[cfg(not(feature = "no_tty_size_please"))]
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
async fn modify0<T: FromServerImageDifferent + FromServerImageMissingTag + From<sqlx::Error>>(
    tx: &mut Transaction<'_, MySql>,
    server_id: u32,
    resource: Option<ServerResourcePayload>,
    build: Option<ServerBuildPayload>,
    ports: Option<Vec<ServerPortEntryPayload>>,
    network: Option<ServerNetworkPayload>,
) -> Result<(), T> {
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
        .await?;
    }
    if let Some(build) = &build {
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
                ServerFileWrite::Write { data } => {
                    query!("INSERT INTO `server_file_writes` (`server`, `file`, `filetype`, `key`, `value`) VALUES (?, ?, 'write', NULL, ?)", server_id, path, BASE64_STANDARD.encode(data))
                        .execute(&mut **tx)
                        .await?;
                }
                ServerFileWrite::JavaProperties { content } => {
                    for (key, value) in content {
                        query!("INSERT INTO `server_file_writes` (`server`, `file`, `filetype`, `key`, `value`) VALUES (?, ?, 'properties', ?, ?)", server_id, path, key, value)
                            .execute(&mut **tx)
                            .await?;
                    }
                }
                ServerFileWrite::Json { set } => {
                    for patch in set {
                        query!("INSERT INTO `server_file_writes` (`server`, `file`, `filetype`, `value`) VALUES (?, ?, 'json', ?)", server_id, path, serde_json::to_string(patch).expect("Unable to serialize patch"))
                            .execute(&mut **tx)
                            .await?;
                    }
                }
                ServerFileWrite::Yaml { set } => {
                    for patch in set {
                        query!("INSERT INTO `server_file_writes` (`server`, `file`, `filetype`, `value`) VALUES (?, ?, 'yaml', ?)", server_id, path, serde_json::to_string(patch).expect("Unable to serialize patch"))
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
                return Err(<T as FromServerImageMissingTag>::from());
            }

            if let Some(default_tag) = query_scalar!(
                "SELECT `default_tag` FROM `images` WHERE `id` = ?",
                image_id
            )
            .fetch_optional(&mut **tx)
            .await?
            {
                if !default_tag.eq(&image.default_tag) {
                    return Err(<T as FromServerImageDifferent>::from());
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
                    return Err(<T as FromServerImageDifferent>::from());
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
    if let Some(ports) = &ports {
        query!("DELETE FROM `server_ports` WHERE `server` = ?", server_id)
            .execute(&mut **tx)
            .await?;
        for port in ports {
            query!("INSERT INTO `server_ports` (`server`, `internal_port`, `external_port`, `external_ip`, `protocol`) VALUES (?, ?, ?, ?, ?)", server_id, &port.internal_port, &port.external_port, &port.external_ip, &port.protocol)
                .execute(&mut **tx)
                .await?;
        }
    }
    if let Some(network) = &network {
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
