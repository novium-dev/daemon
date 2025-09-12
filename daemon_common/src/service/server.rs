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
use super::*;
use crate::service::error_storage_patch_failed::FromStoragePatchFailed;
use crate::user::*;
use crate::utils::{JsonExt as _, io_sync, io_sync_seek_truncate};
use axum::extract::ws::Message;
use chrono::Utc;
use core::error::Error as _;
use crc64::crc64;
use daemon_framework::{JsonVec, WebsocketResponse, define_query_variable};
pub use json_patch::Patch;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::io::SeekFrom;
use tokio::io::{AsyncSeekExt as _, AsyncWriteExt as _};
use tokio::task::JoinError;
use tracing::{debug, error, warn};
use url::Url;

define_error!(server {
    image_different ("Server image differs in database" BAD_REQUEST),
    image_missing_tag ("Server image does not have a tag" BAD_REQUEST),
});

define_query_variable!(write "If write access to the terminal should be granted");
define_query_variable!(cols "The initial value of columns in the terminal");
define_query_variable!(rows "The initial value of rows in the terminal");
define_query_variable!(command "The command to be executed encoded as json string array");

define_query_variable!(precise "If stopping or restarting should be determined. Default: true. If false, the server mutex does not have to be locked.");

#[service(path = "/server", tag = "server")]
pub trait ServerService {
    #[route("", method = PUT)]
    #[error(server(image_different image_missing_tag) storage(location_not_found) sql(failure [sql:*]))]
    #[example("examples/server.create.json")]
    #[doc = "Creates a new server"]
    async fn create_server(
        &self,
        storage: String,
        resource: ServerResourcePayload,
        build: ServerBuildPayload,
        ports: Vec<ServerPortEntryPayload>,
        network: ServerNetworkPayload,
    ) -> Result<u32, _>;

    #[route("/{server_id}", method = PATCH)]
    #[error(server(not_found image_different image_missing_tag) sql(failure [sql:*]))]
    #[example("examples/server.update.json")]
    #[doc = "Updates an existing server"]
    async fn update_server(
        &self,
        server_id: u32,
        resource: Option<ServerResourcePayload>,
        build: Option<ServerBuildPayload>,
        ports: Option<Vec<ServerPortEntryPayload>>,
        network: Option<ServerNetworkPayload>,
    );

    #[route("/{server_id}", method = DELETE)]
    #[error(server(not_found mutex_locked) docker(failure) sql(failure [sql:*]))]
    #[doc = "Deletes an existing server"]
    async fn delete_server(&self, server_id: u32);

    #[route("/{server_id}", method = GET, query = [ precise ])]
    #[error(server(not_found mutex_locked) docker(failure) sql(failure [sql:*]))]
    #[doc = "Gets the status of an existing server"]
    async fn server_status(
        &self,
        server_id: u32,
        precise: Option<bool>,
    ) -> Result<PublicServerState, _>;

    #[route("/{server_id}/start", method = GET)]
    #[error(server(not_found) sql(failure [sql:*]))]
    #[doc = "Starts an existing server"]
    async fn start_server(&self, server_id: u32);

    #[route("/{server_id}/stop", method = GET)]
    #[error(server(not_found) sql(failure [sql:*]))]
    #[doc = "Stops an existing server"]
    async fn stop_server(&self, server_id: u32);

    #[route("/{server_id}/kill", method = GET)]
    #[error(server(not_found) sql(failure [sql:*]))]
    #[doc = "Kills an existing server"]
    async fn kill_server(&self, server_id: u32);

    #[route("/{server_id}/restart", method = GET)]
    #[error(server(not_found) sql(failure [sql:*]))]
    #[doc = "Restarts an existing server"]
    async fn restart_server(&self, server_id: u32);

    #[route("/{server_id}/exec", method = POST)]
    #[error(server(not_found mutex_locked) sql(failure [sql:*]))]
    #[example("examples/server.exec.json")]
    #[doc = "Executes a command inside an running server"]
    async fn exec_server(&self, server_id: u32, cmd: Vec<String>);

    #[route("/{server_id}/terminal", method = WS, query = [ cols, rows, write ])]
    #[error(server(not_found) sql(failure [sql:*]))]
    #[doc = "A websocket for interaction with the terminal"]
    async fn server_terminal(
        &self,
        server_id: u32,
        cols: Option<u16>,
        rows: Option<u16>,
        write: bool,
    ) -> Result<WebsocketResponse, _>;

    #[route(!"/user/server/{server_id}/terminal", method = WS, query = [ timestamp, verify, cols, rows, write ], user)]
    #[error(auth(signature_invalid signature_expired) server(not_found) sql(failure [sql:*]))]
    #[doc = "A websocket for interaction with the terminal (called by the user)"]
    async fn server_terminal_user(
        &self,
        server_id: u32,
        cols: Option<u16>,
        rows: Option<u16>,
        write: bool,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, _>;

    #[route("/{server_id}/stats", method = WS)]
    #[error(server(not_found) sql(failure [sql:*]))]
    #[doc = "A websocket for server stats"]
    async fn server_stats(&self, server_id: u32) -> Result<WebsocketResponse, _>;

    #[route(!"/user/server/{server_id}/stats", method = WS, query = [ timestamp, verify ], user)]
    #[error(auth(signature_invalid signature_expired) server(not_found) sql(failure [sql:*]))]
    #[doc = "A websocket for server stats (called by the user)"]
    async fn server_stats_user(
        &self,
        server_id: u32,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, _>;

    #[route("/{server_id}/exec", method = WS, query = [ cols, rows, command ])]
    #[error(server(not_found) sql(failure [sql:*]))]
    #[doc = "A websocket for server execution"]
    async fn exec_server_interactive(
        &self,
        server_id: u32,
        cols: Option<u16>,
        rows: Option<u16>,
        command: JsonVec<String>,
    ) -> Result<WebsocketResponse, _>;

    #[route(!"/user/server/{server_id}/exec", method = WS, query = [ timestamp, verify, cols, rows, command ], user)]
    #[error(auth(signature_invalid signature_expired) server(not_found) sql(failure [sql:*]))]
    #[doc = "A websocket for server execution (called by the user)"]
    async fn exec_server_interactive_user(
        &self,
        server_id: u32,
        cols: Option<u16>,
        rows: Option<u16>,
        command: JsonVec<String>,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, _>;
}

pub trait ServerServiceUserKeyExtension {
    fn server_terminal_user_url_signed(
        &self,
        server_id: u32,
        cols: Option<u16>,
        rows: Option<u16>,
        write: bool,
    ) -> Url;

    fn server_stats_user_url_signed(&self, server_id: u32) -> Url;

    fn exec_server_interactive_user_url_signed(
        &self,
        server_id: u32,
        cols: Option<u16>,
        rows: Option<u16>,
        command: JsonVec<String>,
    ) -> Url;
}

impl<T: ServerServiceURL + UserKeyProvider> ServerServiceUserKeyExtension for T {
    fn server_terminal_user_url_signed(
        &self,
        server_id: u32,
        cols: Option<u16>,
        rows: Option<u16>,
        write: bool,
    ) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature =
            UserSignature::new_terminal(self.key(), timestamp, self.node_id(), server_id, write);
        self.server_terminal_user_url(server_id, cols, rows, write, timestamp, signature)
    }

    fn server_stats_user_url_signed(&self, server_id: u32) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature = UserSignature::new_stats(self.key(), timestamp, self.node_id(), server_id);
        self.server_stats_user_url(server_id, timestamp, signature)
    }

    fn exec_server_interactive_user_url_signed(
        &self,
        server_id: u32,
        cols: Option<u16>,
        rows: Option<u16>,
        command: JsonVec<String>,
    ) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature =
            UserSignature::new_exec(self.key(), timestamp, self.node_id(), server_id, &command);
        self.exec_server_interactive_user_url(server_id, cols, rows, command, timestamp, signature)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ServerResourcePayload {
    pub cpu: Option<f32>,
    pub ram: Option<u32>,
    pub swap: Option<u32>,
    pub disk: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ServerBuildPayload {
    pub image: ServerBuildImage,

    pub backup_exclude: HashSet<String>,
    pub files_write: HashMap<String, ServerFileWrite>,

    pub cap_add: Vec<String>,
    pub cap_drop: Vec<String>,

    pub environment: HashMap<String, String>,

    pub uid: Option<u16>,
    pub gid: Option<u16>,

    pub stop_command: Option<String>,
    pub stop_timeout: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(no_recursion)]
pub struct ServerBuildImage {
    pub dockerfile: String,
    pub default_tag: Option<String>,
    pub tag: Option<String>,
    pub args: HashMap<String, String>,
    pub parents: Vec<ServerBuildImage>,
    pub base: Option<usize>,
}

impl ServerBuildImage {
    #[must_use]
    pub fn id(&self) -> u64 {
        crc64(0, self.dockerfile().as_bytes())
    }

    #[must_use]
    pub fn used_tag(&self) -> Option<String> {
        match (&self.default_tag, &self.tag) {
            (_, Some(tag)) | (Some(tag), _) => Some(tag.clone()),
            _ => None,
        }
    }

    #[must_use]
    pub fn dockerfile(&self) -> String {
        match self.base {
            Some(base) => {
                let Some(parent) = self.parents.get(base) else {
                    return self.dockerfile.clone();
                };
                let Some(tag) = parent.used_tag() else {
                    return self.dockerfile.clone();
                };
                if self.dockerfile.contains("{BASE}") {
                    self.dockerfile
                        .replace("{BASE}", &format!("{}:{}", crate::IMAGE_NAME, tag))
                } else {
                    format!("FROM {}:{}\n{}", crate::IMAGE_NAME, tag, self.dockerfile)
                }
            }
            None => self.dockerfile.clone(),
        }
    }

    #[must_use]
    pub fn all(&self) -> Vec<&Self> {
        let mut vec = Vec::with_capacity(1);
        for entries in &self.parents {
            vec.extend(entries.all());
        }
        vec.push(self);
        vec
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ServerNetworkPayload {
    pub internet_access: bool,
    pub networks: Vec<NetworkAttachment>,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct NetworkAttachment {
    pub network: u32,
    pub ipv4: u8,
    pub ipv6: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ServerPortEntryPayload {
    pub internal_port: u16,
    pub external_port: u16,
    pub external_ip: String,
    pub protocol: String,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({"label": "Running", "healthcheck": null, "start": false, "stop": true, "kill": false, "time": 0}))]
pub struct PublicServerState {
    pub label: String,
    pub healthcheck: Option<String>,
    pub start: bool,
    pub stop: bool,
    pub kill: bool,
    pub time: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "filetype", rename_all = "snake_case")]
pub enum ServerFileWrite {
    Write { data: Vec<u8> },
    JavaProperties { content: HashMap<String, String> },
    Json { set: Vec<PatchSet> },
    Yaml { set: Vec<PatchSet> },
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct PatchSet {
    pub condition: Patch,
    pub patch: PatchEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "patchtype", rename_all = "snake_case")]
pub enum PatchEntry {
    Normal { patch: Patch },
    Merge { patch: Value },
}

impl ServerFileWrite {
    /// Not Cancel Safe
    pub async fn apply<
        E: From<io::Error> + FromStorageQuotaReached + FromStoragePatchFailed + Send + 'static,
    >(
        &self,
        mut file: tokio::fs::File,
    ) -> Result<(), E> {
        use std::io::Seek as _;

        match self {
            Self::Write { data } => {
                file.set_len(0)
                    .await
                    .inspect_err(|err| error!("Failed to truncate file: {:?}", err))?;
                file.seek(SeekFrom::Start(0))
                    .await
                    .inspect_err(|err| error!("Failed to seek file: {:?}", err))?;
                file.write_all(data)
                    .await
                    .inspect_err(|err| {
                        if err.kind() == ErrorKind::QuotaExceeded {
                            debug!("Failed to write data: {:?}", err);
                        } else {
                            error!("Failed to write data: {:?}", err);
                        }
                    })
                    .map_err(|err| {
                        if err.kind() == ErrorKind::QuotaExceeded {
                            <E as FromStorageQuotaReached>::from()
                        } else {
                            <E as From<io::Error>>::from(err)
                        }
                    })?;
            }
            Self::JavaProperties { content } => {
                let content = content.clone();
                let mut file = file.into_std().await;
                tokio::task::spawn_blocking(move || {
                    let mut current = java_properties::read(&mut file)
                        .inspect_err(|err| debug!("Failed to read properties file: {:?}", err))
                        .unwrap_or_default();
                    for (key, val) in content {
                        current.insert(key, val);
                    }

                    file.set_len(0)
                        .inspect_err(|err| error!("Failed to truncate file: {:?}", err))?;
                    file.seek(SeekFrom::Start(0))
                        .inspect_err(|err| error!("Failed to seek file: {:?}", err))?;
                    java_properties::write(&mut file, &current)
                        .inspect_err(|err| warn!("Failed to write properties: {:?}", err)) // only warning, Potential QUOTA_EXCEEDED
                        .map_err(|err| {
                            if let Some(err) = err.source()
                                && let Some(err) = err.downcast_ref::<io::Error>()
                                && err.kind() == ErrorKind::QuotaExceeded
                            {
                                <E as FromStorageQuotaReached>::from()
                            } else {
                                <E as From<io::Error>>::from(io::Error::other(err))
                            }
                        })?;
                    Ok::<_, E>(())
                })
                .await
                .inspect_err(|err| error!("Failed to write properties (task): {:?}", err))
                .map_err(<io::Error as From<JoinError>>::from)??;
            }
            Self::Json { set } => {
                let (file, mut parsed): (_, serde_json::Value) = io_sync(
                    |read| {
                        serde_json::from_reader(read)
                            .map_err(<io::Error as From<serde_json::Error>>::from)
                    },
                    file,
                )
                .await?;

                for patch in set {
                    let mut copy = parsed.clone();
                    if json_patch::patch(&mut copy, &patch.condition).is_err() {
                        continue;
                    }
                    match &patch.patch {
                        PatchEntry::Normal { patch } => {
                            json_patch::patch(&mut parsed, patch)
                                .map_err(|_err| <E as FromStoragePatchFailed>::from())?;
                        }
                        PatchEntry::Merge { patch } => {
                            json_patch::merge(&mut parsed, patch);
                        }
                    }
                }

                io_sync_seek_truncate(
                    move |write| {
                        serde_json::to_writer_pretty(write, &parsed)
                            .map_err(<io::Error as From<serde_json::Error>>::from)
                    },
                    file,
                )
                .await
                .map_err(|err| {
                    if err.kind() == ErrorKind::QuotaExceeded {
                        <E as FromStorageQuotaReached>::from()
                    } else {
                        <E as From<io::Error>>::from(err)
                    }
                })?;
            }
            Self::Yaml { set } => {
                let (file, mut parsed): (_, serde_json::Value) = io_sync(
                    |read| {
                        serde_yaml_ng::from_reader(read)
                            .map_err(<io::Error as From<serde_yaml_ng::Error>>::from)
                            .map(serde_json::Value::from_yaml)
                    },
                    file,
                )
                .await?;

                for patch in set {
                    let mut copy = parsed.clone();
                    if json_patch::patch(&mut copy, &patch.condition).is_err() {
                        continue;
                    }
                    match &patch.patch {
                        PatchEntry::Normal { patch } => {
                            json_patch::patch(&mut parsed, patch)
                                .map_err(|_err| <E as FromStoragePatchFailed>::from())?;
                        }
                        PatchEntry::Merge { patch } => {
                            json_patch::merge(&mut parsed, patch);
                        }
                    }
                }

                io_sync_seek_truncate(
                    move |write| {
                        serde_yaml_ng::to_writer(write, &parsed.to_yaml())
                            .map_err(<io::Error as From<serde_yaml_ng::Error>>::from)
                    },
                    file,
                )
                .await
                .map_err(|err| {
                    if err.kind() == ErrorKind::QuotaExceeded {
                        <E as FromStorageQuotaReached>::from()
                    } else {
                        <E as From<io::Error>>::from(err)
                    }
                })?;
            }
        }
        Ok(())
    }
}

#[derive(Default, Copy, Clone, Serialize, Deserialize)]
#[must_use]
pub struct ServerStats {
    pub cpu: Option<f64>,      // in cpus
    pub ram: Option<u64>,      // in bytes
    pub swap: Option<u64>,     // in bytes
    pub disk: Option<u64>,     // in bytes
    pub cpu_max: Option<f32>,  // in cpus
    pub ram_max: Option<u32>,  // in mib
    pub swap_max: Option<u32>, // in mib
    pub disk_max: Option<u64>, // in mib
}

impl ServerStats {
    pub const fn with_max(
        self,
        cpu: Option<f32>,
        ram: Option<u32>,
        swap: Option<u32>,
        disk: Option<u64>,
    ) -> Self {
        Self {
            cpu_max: cpu,
            ram_max: ram,
            swap_max: swap,
            disk_max: disk,
            ..self
        }
    }

    pub const fn with_disk(self, disk: Option<u64>) -> Self {
        Self { disk, ..self }
    }

    #[must_use]
    pub fn as_websocket_message(&self) -> Message {
        const STATS_MAGIC: &str = "STATS!";
        Message::Text(
            format!(
                "{STATS_MAGIC}{}",
                serde_json::to_string(self)
                    .expect("Unable to serialize stats to websocket message")
            )
            .into(),
        )
    }
}

#[cfg(feature = "docker")]
impl From<bollard::models::ContainerStatsResponse> for ServerStats {
    #[expect(
        clippy::cast_precision_loss,
        reason = "cpu and ram doesn't have to be precise"
    )]
    fn from(value: bollard::models::ContainerStatsResponse) -> Self {
        let cpu_stats = value.cpu_stats.unwrap_or_default();
        let precpu_stats = value.precpu_stats.unwrap_or_default();
        let memory_stats = value.memory_stats.unwrap_or_default();

        let system_delta = cpu_stats
            .system_cpu_usage
            .unwrap_or_default()
            .saturating_sub(precpu_stats.system_cpu_usage.unwrap_or_default())
            as f64;
        let cpu_delta = cpu_stats
            .cpu_usage
            .unwrap_or_default()
            .total_usage
            .unwrap_or_default()
            .saturating_sub(
                precpu_stats
                    .cpu_usage
                    .unwrap_or_default()
                    .total_usage
                    .unwrap_or_default(),
            ) as f64;

        Self {
            cpu: (cpu_stats.online_cpus.is_some() && system_delta > 0f64 && cpu_delta > 0f64).then(
                || {
                    ((cpu_delta / system_delta)
                        * f64::from(cpu_stats.online_cpus.unwrap_or_default())
                        * 10000f64)
                        .round()
                        / 100f64
                },
            ),
            ram: memory_stats.usage,
            swap: None, // TODO
            ..Self::default()
        }
    }
}
