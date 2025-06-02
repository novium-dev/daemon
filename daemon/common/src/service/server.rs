use super::*;
use crate::user::*;
use axum::extract::ws::Message;
use chrono::Utc;
use crc64::crc64;
use daemon_framework::{JsonVec, WebsocketResponse, define_query_variable, ts};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_yaml::Mapping;
use std::collections::{HashMap, HashSet};
use std::io::SeekFrom;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
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

    // TODO: Move to FilesService
    #[route("/{server_id}/wipe", method = GET)]
    #[error(storage(not_found mutex_locked failure [io:*]) sql(failure [sql:*]))]
    #[doc = "Deletes all files in an existing server"]
    async fn wipe_server(&self, server_id: u32);

    #[route(!"/user/server/{server_id}/terminal", method = WS, query = [ timestamp, verify, cols, rows, write ], user)]
    #[error(auth(signature_invalid signature_expired) server(not_found) sql(failure [sql:*]))]
    #[doc = "A websocket for interaction with the terminal"]
    async fn server_terminal(
        &self,
        server_id: u32,
        cols: Option<u32>,
        rows: Option<u32>,
        write: bool,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, _>;

    #[route(!"/user/server/{server_id}/stats", method = WS, query = [ timestamp, verify ], user)]
    #[error(auth(signature_invalid signature_expired) server(not_found) sql(failure [sql:*]))]
    #[doc = "A websocket for server stats"]
    async fn server_stats(
        &self,
        server_id: u32,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, _>;

    #[route(!"/user/server/{server_id}/exec", method = WS, query = [ timestamp, verify, cols, rows, command ], user)]
    #[error(auth(signature_invalid signature_expired) server(not_found) sql(failure [sql:*]))]
    #[doc = "A websocket for server execution"]
    async fn exec_server_interactive(
        &self,
        server_id: u32,
        cols: Option<u32>,
        rows: Option<u32>,
        command: JsonVec<String>,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, _>;
}

pub trait ServerServiceUserKeyExtension {
    fn server_terminal_url_signed(
        &self,
        server_id: u32,
        cols: Option<u32>,
        rows: Option<u32>,
        write: bool,
    ) -> Url;

    fn server_stats_url_signed(&self, server_id: u32) -> Url;

    fn exec_server_interactive_url_signed(
        &self,
        server_id: u32,
        cols: Option<u32>,
        rows: Option<u32>,
        command: JsonVec<String>,
    ) -> Url;

    #[cfg(feature = "ts")]
    fn visit_funcs(visitor: &mut daemon_framework::ts::TSClassFunctionVisitor);
}

impl<T: ServerServiceURL + UserKeyProvider> ServerServiceUserKeyExtension for T {
    fn server_terminal_url_signed(
        &self,
        server_id: u32,
        cols: Option<u32>,
        rows: Option<u32>,
        write: bool,
    ) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature =
            UserSignature::new_terminal(self.key(), timestamp, self.node_id(), server_id, write);
        self.server_terminal_url(server_id, cols, rows, write, timestamp, signature)
    }

    fn server_stats_url_signed(&self, server_id: u32) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature = UserSignature::new_stats(self.key(), timestamp, self.node_id(), server_id);
        self.server_stats_url(server_id, timestamp, signature)
    }

    fn exec_server_interactive_url_signed(
        &self,
        server_id: u32,
        cols: Option<u32>,
        rows: Option<u32>,
        command: JsonVec<String>,
    ) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature =
            UserSignature::new_exec(self.key(), timestamp, self.node_id(), server_id, &command);
        self.exec_server_interactive_url(server_id, cols, rows, command, timestamp, signature)
    }

    #[cfg(feature = "ts")]
    fn visit_funcs(visitor: &mut daemon_framework::ts::TSClassFunctionVisitor) {
        visitor.visit_start(true, "serverTerminalUrlSigned");
        visitor.visit_argument::<u32>("serverId");
        visitor.visit_argument::<Option<u32>>("cols");
        visitor.visit_argument::<Option<u32>>("rows");
        visitor.visit_argument::<bool>("write");
        visitor.visit_end_result_custom(r#"const timestamp: bigint = BigInt(Date.now());
const signature = await UserSignature.newTerminalAsync(this.userKey, timestamp, this.identifier, serverId, write);
return this.serverTerminalUrl(serverId, cols, rows, write, timestamp, signature);"#, "URL");

        visitor.visit_start(true, "serverStatsUrlSigned");
        visitor.visit_argument::<u32>("serverId");
        visitor.visit_end_result_custom(r#"const timestamp: bigint = BigInt(Date.now());
const signature = await UserSignature.newStatsAsync(this.userKey, timestamp, this.identifier, serverId);
return this.serverStatsUrl(serverId, timestamp, signature);"#, "URL");

        visitor.visit_start(true, "execServerInteractiveUrlSigned");
        visitor.visit_argument::<u32>("serverId");
        visitor.visit_argument::<Option<u32>>("cols");
        visitor.visit_argument::<Option<u32>>("rows");
        visitor.visit_argument::<JsonVec<String>>("command");
        visitor.visit_end_result_custom(r#"const timestamp: bigint = BigInt(Date.now());
const signature = await UserSignature.newExecAsync(this.userKey, timestamp, this.identifier, serverId, command);
return this.execServerInteractiveUrl(serverId, cols, rows, command, timestamp, signature);"#, "URL");
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[ts]
pub struct ServerResourcePayload {
    pub cpu: Option<f32>,
    pub ram: Option<u32>,
    pub swap: Option<u32>,
    pub disk: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[ts]
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
#[ts]
pub struct ServerBuildImage {
    pub dockerfile: String,
    pub default_tag: Option<String>,
    pub tag: Option<String>,
    pub args: HashMap<String, String>,
    pub parents: Vec<ServerBuildImage>,
    pub base: Option<usize>,
}

impl ServerBuildImage {
    pub fn id(&self) -> u64 {
        crc64(0, self.dockerfile().as_bytes())
    }

    pub fn used_tag(&self) -> Option<String> {
        match (&self.default_tag, &self.tag) {
            (_, Some(tag)) | (Some(tag), _) => Some(tag.clone()),
            _ => None,
        }
    }

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

    pub fn all(&self) -> Vec<&ServerBuildImage> {
        let mut vec = Vec::with_capacity(1);
        for entries in &self.parents {
            vec.extend(entries.all());
        }
        vec.push(self);
        vec
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[ts]
pub struct ServerNetworkPayload {
    pub internet_access: bool,
    pub networks: Vec<NetworkAttachment>,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[ts]
pub struct NetworkAttachment {
    pub network: u32,
    pub ipv4: u8,
    pub ipv6: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[ts]
pub struct ServerPortEntryPayload {
    pub internal_port: u16,
    pub external_port: u16,
    pub external_ip: String,
    pub protocol: String,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({"label": "Running", "healthcheck": null, "start": false, "stop": true, "kill": false, "time": 0}))]
#[ts]
pub struct PublicServerState {
    pub label: String,
    pub healthcheck: Option<String>,
    pub start: bool,
    pub stop: bool,
    pub kill: bool,
    pub time: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[ts]
#[serde(tag = "filetype")]
pub enum ServerFileWrite {
    #[serde(rename = "write")]
    WRITE { data: Vec<u8> },
    #[serde(rename = "properties")]
    PROPERTIES { content: HashMap<String, String> },
    #[serde(rename = "json")]
    JSON {
        content: HashMap<String, serde_json::Value>,
    },
    #[serde(rename = "yaml")]
    YAML {
        content: HashMap<String, serde_json::Value>,
    },
}

impl ServerFileWrite {
    /// Not Cancel Safe
    pub async fn apply<E: From<io::Error> + FromStorageQuotaReached + Send + 'static>(
        &self,
        mut file: tokio::fs::File,
    ) -> Result<(), E> {
        use std::io::Seek;

        match self {
            ServerFileWrite::WRITE { data } => {
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
                            debug!("Failed to write data: {:?}", err)
                        } else {
                            error!("Failed to write data: {:?}", err)
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
            ServerFileWrite::PROPERTIES { content } => {
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
                        .map_err(io::Error::other)?;
                    Ok::<_, E>(())
                })
                .await
                .inspect_err(|err| error!("Failed to write properties (task): {:?}", err))
                .map_err(<io::Error as From<JoinError>>::from)??;
            }
            ServerFileWrite::JSON { content } => {
                let mut file = file.into_std().await;
                let content = content.clone();
                let (mut value, mut file) =
                    tokio::task::spawn_blocking(move || match serde_json::from_reader(&mut file) {
                        Ok(val) => Ok::<_, E>((val, file)),
                        Err(err) if err.is_io() => {
                            let kind = err.io_error_kind().unwrap();
                            if kind == ErrorKind::QuotaExceeded {
                                debug!("Failed to read json file: {:?}", err);
                                Err(<E as FromStorageQuotaReached>::from())
                            } else {
                                error!("Failed to read json file: {:?}", err);
                                Err(<E as From<io::Error>>::from(io::Error::new(
                                    err.io_error_kind().unwrap(),
                                    err,
                                )))
                            }
                        }
                        Err(_err) => Ok((Value::Object(Map::new()), file)),
                    })
                    .await
                    .inspect_err(|err| error!("Failed to read json (task): {:?}", err))
                    .map_err(<io::Error as From<JoinError>>::from)??;

                for (key, val) in content {
                    let components: Vec<String> = key.split(".").map(String::from).collect();
                    let mut current = &mut value;
                    for comp in components {
                        match &current {
                            Value::Array(array) => {
                                let index = comp.parse::<usize>().map_err(io::Error::other)?;
                                if index < array.len() {
                                    current = &mut current.as_array_mut().unwrap()[index];
                                } else {
                                    while index >= current.as_array_mut().unwrap().len() {
                                        current.as_array_mut().unwrap().push(
                                            serde_json::Value::Object(serde_json::Map::new()),
                                        );
                                    }
                                    current = &mut current.as_array_mut().unwrap()[index];
                                }
                            }
                            Value::Object(obj) => {
                                if obj.contains_key(&comp) {
                                    current =
                                        current.as_object_mut().unwrap().get_mut(&comp).unwrap();
                                } else {
                                    current.as_object_mut().unwrap().insert(
                                        comp.clone(),
                                        serde_json::Value::Object(serde_json::Map::new()),
                                    );
                                    current =
                                        current.as_object_mut().unwrap().get_mut(&comp).unwrap();
                                }
                            }
                            _ => {
                                *current = serde_json::Value::Object(Map::new());
                                if current.as_object_mut().unwrap().contains_key(&comp) {
                                    current =
                                        current.as_object_mut().unwrap().get_mut(&comp).unwrap();
                                } else {
                                    current.as_object_mut().unwrap().insert(
                                        comp.clone(),
                                        serde_json::Value::Object(serde_json::Map::new()),
                                    );
                                    current =
                                        current.as_object_mut().unwrap().get_mut(&comp).unwrap();
                                }
                            }
                        }
                    }
                    *current = val;
                }
                tokio::task::spawn_blocking(move || {
                    file.set_len(0)
                        .inspect_err(|err| error!("Failed to truncate file: {:?}", err))?;
                    file.seek(SeekFrom::Start(0))
                        .inspect_err(|err| error!("Failed to seek file: {:?}", err))?;
                    serde_json::to_writer_pretty(&mut file, &value)
                        .inspect_err(|err| warn!("Failed to write json: {:?}", err))
                        .map_err(io::Error::other)
                })
                .await
                .inspect_err(|err| error!("Failed to write json (task): {:?}", err))
                .map_err(<io::Error as From<JoinError>>::from)??;
            }
            ServerFileWrite::YAML { content } => {
                let mut file = file.into_std().await;
                let content = content.clone();
                let (mut value, mut file): (serde_yaml::Value, _) =
                    tokio::task::spawn_blocking(move || {
                        serde_yaml::from_reader(&mut file)
                            .inspect_err(|err| debug!("Failed to read yaml: {:?}", err))
                            .map_err(|err| <E as From<io::Error>>::from(io::Error::other(err)))
                            .map(|val: serde_yaml::Value| (val, file))
                    })
                    .await
                    .inspect_err(|err| error!("Failed to read yaml (task): {:?}", err))
                    .map_err(<io::Error as From<JoinError>>::from)??;

                for (key, val) in content {
                    let components: Vec<String> = key.split(".").map(String::from).collect();
                    let mut current = &mut value;
                    for comp in components {
                        match &current {
                            serde_yaml::Value::Sequence(array) => {
                                let index = comp.parse::<usize>().map_err(|err| {
                                    <E as From<io::Error>>::from(io::Error::other(err))
                                })?;
                                if index < array.len() {
                                    current = &mut current.as_sequence_mut().unwrap()[index];
                                } else {
                                    while index >= current.as_sequence_mut().unwrap().len() {
                                        current.as_sequence_mut().unwrap().push(
                                            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
                                        );
                                    }
                                    current = &mut current.as_sequence_mut().unwrap()[index];
                                }
                            }
                            serde_yaml::Value::Mapping(obj) => {
                                if obj.contains_key(&comp) {
                                    current =
                                        current.as_mapping_mut().unwrap().get_mut(&comp).unwrap();
                                } else {
                                    current.as_mapping_mut().unwrap().insert(
                                        serde_yaml::Value::String(comp.clone()),
                                        serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
                                    );
                                    current =
                                        current.as_mapping_mut().unwrap().get_mut(&comp).unwrap();
                                }
                            }
                            _ => {
                                *current = serde_yaml::Value::Mapping(Mapping::new());
                                if current.as_mapping_mut().unwrap().contains_key(&comp) {
                                    current =
                                        current.as_mapping_mut().unwrap().get_mut(&comp).unwrap();
                                } else {
                                    current.as_mapping_mut().unwrap().insert(
                                        serde_yaml::Value::String(comp.clone()),
                                        serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
                                    );
                                    current =
                                        current.as_mapping_mut().unwrap().get_mut(&comp).unwrap();
                                }
                            }
                        }
                    }
                    *current = serde_json::from_value(val)
                        .inspect_err(|err| warn!("Failed to convert json to yaml: {:?}", err))
                        .map_err(|err| <E as From<io::Error>>::from(io::Error::other(err)))?;
                }
                tokio::task::spawn_blocking(move || {
                    file.set_len(0)
                        .inspect_err(|err| error!("Failed to truncate file: {:?}", err))?;
                    file.seek(SeekFrom::Start(0))
                        .inspect_err(|err| error!("Failed to seek file: {:?}", err))?;
                    serde_yaml::to_writer(&mut file, &value)
                        .inspect_err(|err| warn!("Failed to write yaml: {:?}", err))
                        .map_err(io::Error::other)
                })
                .await
                .map_err(<io::Error as From<JoinError>>::from)??;
            }
        }
        Ok(())
    }
}

#[derive(Copy, Clone, Serialize, Deserialize)]
pub struct ServerStats {
    pub cpu: f32,
    pub ram: f32,
    pub swap: f32,
    pub disk: i64,
    pub cpu_max: f32,
    pub ram_max: i64,
    pub swap_max: i64,
    pub disk_max: i64,
}

impl Default for ServerStats {
    fn default() -> Self {
        Self {
            cpu: -1f32,
            ram: -1f32,
            swap: -1f32,
            disk: -1,
            cpu_max: -1f32,
            ram_max: -1,
            swap_max: -1,
            disk_max: -1,
        }
    }
}

impl ServerStats {
    pub fn with_max(self, cpu: f32, ram: i64, swap: i64, disk: i64) -> Self {
        Self {
            cpu_max: cpu,
            ram_max: ram,
            swap_max: swap,
            disk_max: disk,
            ..self
        }
    }

    pub fn with_disk(self, disk: i64) -> Self {
        Self { disk, ..self }
    }
}

#[cfg(feature = "docker")]
impl From<bollard::models::ContainerStatsResponse> for ServerStats {
    fn from(value: bollard::models::ContainerStatsResponse) -> Self {
        let mut cpu = -1f32;
        let mut ram = -1f32;
        let swap = -1f32;

        let cpu_stats = value.cpu_stats.unwrap_or_default();
        let precpu_stats = value.precpu_stats.unwrap_or_default();
        let memory_stats = value.memory_stats.unwrap_or_default();

        let system_delta = cpu_stats.system_cpu_usage.unwrap_or_default() as f64
            - precpu_stats.system_cpu_usage.unwrap_or_default() as f64;
        let cpu_delta = cpu_stats
            .cpu_usage
            .unwrap_or_default()
            .total_usage
            .unwrap_or_default() as f64
            - precpu_stats
                .cpu_usage
                .unwrap_or_default()
                .total_usage
                .unwrap_or_default() as f64;

        if cpu_stats.online_cpus.is_some() && system_delta > 0f64 && cpu_delta > 0f64 {
            cpu = (((cpu_delta / system_delta)
                * cpu_stats.online_cpus.unwrap_or_default() as f64
                * 10000f64)
                .round()
                / 100f64) as f32;
        }

        if let Some(usage) = memory_stats.usage {
            let mut usage = usage as f64;
            usage /= 1024f64;
            usage /= 1024f64;
            usage *= 100f64;
            usage = usage.round();
            usage /= 100f64;
            if usage <= f32::MAX as f64 {
                ram = usage as f32;
            }
        }

        Self {
            cpu,
            ram,
            swap,
            ..Self::default()
        }
    }
}

impl ServerStats {
    pub fn as_websocket_message(&self) -> Message {
        const STATS_MAGIC: &str = "STATS!";
        Message::Text(
            format!(
                "{}{}",
                STATS_MAGIC.to_owned(),
                serde_json::to_string(self).unwrap()
            )
            .into(),
        )
    }
}
