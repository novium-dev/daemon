use crate::config::DaemonConfig;
use crate::schema::FetchedServer;
use crate::storage::StorageIdentifiers;
use crate::utils::{from_mib, ChannelWriteData};
use crate::{storage, utils, DaemonState, ServerData};
#[cfg(feature = "websocket")]
use axum::extract::ws::Message;
use bollard::container::{
    AttachContainerOptions, Config, CreateContainerOptions, ListContainersOptions,
    NetworkingConfig, ResizeContainerTtyOptions, Stats, StatsOptions,
};
use bollard::image::{BuildImageOptions, ListImagesOptions};
use bollard::models::{HealthStatusEnum, PortBinding};
use bollard::service::HostConfig;
use bollard::Docker;
use futures::stream::StreamExt;
use log::warn;
use serde::{Serialize, Serializer};
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::future::Future;
use std::io;
use std::io::ErrorKind;
use std::ops::{Add, Deref, DerefMut};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tar::{Builder, Header};
use termion::color::Fg;
use termion::{clear, color};
use tokio::sync::broadcast::Sender;
use tokio::sync::Mutex;
use tokio::time::Instant;

pub(crate) const TERMINAL_BUFFER_LIMIT: usize = 1024 * 1024;

impl FetchedServer {
    pub(crate) fn prefix() -> String {
        format!("{}[Daemon] ", Fg(color::Yellow))
    }

    pub(crate) fn console(sender: &Sender<ChannelWriteData>, buffer: &mut Vec<u8>, msg: String) {
        let bytes = msg.into_bytes();
        if (buffer.len() + bytes.len()) > TERMINAL_BUFFER_LIMIT {
            let remove = TERMINAL_BUFFER_LIMIT - (buffer.len() + bytes.len());
            buffer.drain(0..remove);
        }
        buffer.extend_from_slice(&bytes);
        let _ = sender.send(ChannelWriteData::Terminal(bytes));
    }

    pub(crate) fn log(sender: &Sender<ChannelWriteData>, buffer: &mut Vec<u8>, msg: &str) {
        Self::console(
            sender,
            buffer,
            format!("\r{}{}{}\r\n", Self::prefix(), msg, Fg(color::Reset)),
        );
    }

    pub(crate) async fn prepare_image(
        &self,
        docker: &Docker,
        buffer: &mut Vec<u8>,
        server_data: &mut ServerData,
    ) -> Result<String, ServerStartError> {
        let name = format!("{}:{}", crate::IMAGE_NAME, self.build_identifier);

        let images = docker
            .list_images(Some(ListImagesOptions {
                filters: HashMap::from([("reference".to_owned(), vec![name.clone()])]),
                ..ListImagesOptions::default()
            }))
            .await
            .map_err(ServerStartError::Docker)?;

        if images.is_empty() {
            Self::log(&server_data.sender, buffer, "Building Image...");

            let dockerfile = {
                let dockerfile = self.build_dockerfile.as_bytes();
                let mut header = Header::new_gnu();
                header.set_path("Dockerfile").unwrap();
                header.set_size(dockerfile.len() as u64);
                header.set_cksum();
                let mut ar = Builder::new(Vec::new());
                ar.append(&header, dockerfile)
                    .map_err(ServerStartError::IO)?;
                ar.finish().map_err(ServerStartError::IO)?;
                ar.into_inner().map_err(ServerStartError::IO)?
            };

            let mut image = docker.build_image(
                BuildImageOptions {
                    buildargs: self.build_args.clone(),
                    t: name.clone(),
                    rm: true,
                    ..BuildImageOptions::default()
                },
                None,
                Some(bytes::Bytes::from(dockerfile)),
            );

            let mut last_progress = Vec::new();
            let mut progress_map = HashMap::new();
            let mut sort_vec = Vec::new();
            while let Some(progress) = image.next().await {
                let progress = progress.map_err(ServerStartError::Docker)?;

                if let Some(stream) = progress.stream {
                    if !progress_map.is_empty() {
                        progress_map.clear();
                    }
                    Self::console(&server_data.sender, buffer, stream.replace('\n', "\r\n"));
                // It doesn't matter if there could be two \r after this
                } else if let Some(id) = progress.id {
                    if !progress_map.contains_key(&id) {
                        sort_vec.push(id.clone());
                    }
                    progress_map.insert(id, (progress.progress, progress.status));

                    let mut msg = format!(
                        "{}\r\n",
                        termion::cursor::Up(if last_progress.is_empty() {
                            0
                        } else {
                            last_progress.len() as u16 + 1
                        })
                    );

                    last_progress.clear();
                    for (id, (progress, status)) in sort_vec
                        .iter()
                        .map(|key| (key, progress_map.get(key).unwrap()))
                    {
                        let mut msg_intermediate = String::new();
                        msg_intermediate += clear::CurrentLine.as_ref();
                        msg_intermediate += id;
                        if let Some(status) = status {
                            msg_intermediate += ": ";
                            msg_intermediate += status;
                            if let Some(progress) = progress {
                                msg_intermediate += " ";
                                msg_intermediate += progress;
                            }
                            last_progress.push(msg_intermediate);
                        }
                    }
                    for line in &last_progress {
                        msg += line;
                        msg += "\r\n";
                    }
                    if !last_progress.is_empty() {
                        Self::console(&server_data.sender, buffer, msg);
                    }
                }
            }
        }
        Ok(name)
    }

    pub(crate) async fn state(
        id: u32,
        docker: &Docker,
        server_data: &ServerData,
    ) -> Result<ServerState, bollard::errors::Error> {
        let list = docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: HashMap::from([("name".to_owned(), vec![Self::container_name(id)])]),
                ..ListContainersOptions::default()
            }))
            .await?;
        if list.is_empty() {
            Ok(ServerState::Stopped)
        } else {
            let inspect = docker
                .inspect_container(&Self::container_name(id), None)
                .await?;
            let state = inspect.state.unwrap();
            Ok(if !state.running.unwrap() {
                ServerState::Stopped
            } else if server_data.stopping {
                if server_data.restart {
                    ServerState::Restarting
                } else {
                    ServerState::Stopping
                }
            } else if let Some(health) = state.health {
                match health.status.unwrap() {
                    HealthStatusEnum::EMPTY | HealthStatusEnum::NONE => ServerState::Running,
                    HealthStatusEnum::STARTING => ServerState::Starting,
                    HealthStatusEnum::HEALTHY => ServerState::Running,
                    HealthStatusEnum::UNHEALTHY => ServerState::Unhealthy,
                }
            } else {
                ServerState::Running
            })
        }
    }

    pub(crate) fn container_name(id: u32) -> String {
        format!("{}{}", crate::CONTAINER_PREFIX, id)
    }

    pub(crate) async fn start(
        &self,
        config: &DaemonConfig,
        docker: Arc<Docker>,
        server_data: &mut ServerData,
        server_data_arc: Arc<Mutex<ServerData>>,
        state: DaemonState,
    ) -> Result<(), ServerStartError> {
        if !Self::state(self.id, &docker, server_data)
            .await
            .map_err(ServerStartError::Docker)?
            .start()
        {
            return Err(ServerStartError::AlreadyRunning); // Containers are created with delete-on-exit.
        }

        server_data.stopping = false;
        server_data.restart = false;

        let buffer = server_data.buffer.clone();
        let mut buffer = buffer.lock().await;
        buffer.clear();
        Self::log(
            &server_data.sender,
            buffer.deref_mut(),
            "Preparing Server...",
        );

        let image = self
            .clone()
            .prepare_image(&docker, buffer.deref_mut(), server_data)
            .await?;

        let mut binds = Vec::with_capacity(1);
        binds.push(format!(
            "{}:/work",
            storage::ensure_exists(config, StorageIdentifiers::Primary(self.id), self.disk)
                .await
                .map_err(ServerStartError::Storage)?
        ));

        let networks = HashMap::new();

        let mut ports = HashMap::new();
        for port in &self.ports {
            let key = format!("{}/{}", port.internal_port, port.protocol);
            if !ports.contains_key(&key) {
                ports.insert(key.clone(), Some(Vec::new()));
            }
            ports
                .get_mut(&key)
                .unwrap()
                .as_mut()
                .unwrap()
                .push(PortBinding {
                    host_ip: Some(port.external_ip.clone()),
                    host_port: Some(port.external_port.to_string()),
                })
        }

        Self::log(
            &server_data.sender,
            buffer.deref_mut(),
            "Creating Environment...",
        );

        docker
            .create_container(
                Some(CreateContainerOptions {
                    name: Self::container_name(self.id),
                    platform: None,
                }),
                Config {
                    tty: Some(true),
                    open_stdin: Some(true),

                    image: Some(image),

                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    attach_stdin: Some(true),

                    env: Some(vec!["TERM=xterm-256color".to_owned()]),

                    networking_config: Some(NetworkingConfig {
                        endpoints_config: networks,
                    }),

                    exposed_ports: Some(
                        ports
                            .keys()
                            .map(|str| (str.clone(), HashMap::new()))
                            .collect(),
                    ),

                    host_config: Some(HostConfig {
                        auto_remove: Some(true),

                        binds: Some(binds),

                        port_bindings: Some(ports),

                        cap_add: Some(self.cap_add.clone()),
                        cap_drop: Some(self.cap_drop.clone()),

                        cpu_period: self.cpu.map(|_| 100000),
                        cpu_quota: self.cpu.map(|cpu| (100000.0 * cpu) as i64),
                        memory: self.ram.map(utils::from_mib),
                        memory_swap: self.ram.map(|data| {
                            if let Some(swap) = self.swap {
                                from_mib(data + swap)
                            } else {
                                -1
                            }
                        }),

                        console_size: Some(vec![
                            server_data.size.0 as i32,
                            server_data.size.1 as i32,
                        ]),

                        network_mode: Some("bridge".to_owned()),

                        ..HostConfig::default()
                    }),

                    ..Config::default()
                },
            )
            .await
            .map_err(ServerStartError::Docker)?;

        Self::log(
            &server_data.sender,
            buffer.deref_mut(),
            "Starting Server...",
        );

        docker
            .start_container::<String>(&Self::container_name(self.id), None)
            .await
            .map_err(ServerStartError::Docker)?;

        let result = docker
            .attach_container::<String>(
                &Self::container_name(self.id),
                Some(AttachContainerOptions {
                    logs: Some(true),
                    stderr: Some(true),
                    stdout: Some(true),
                    stdin: Some(true),
                    stream: Some(true),
                    ..AttachContainerOptions::default()
                }),
            )
            .await
            .map_err(ServerStartError::Docker)?;
        *server_data.attach_write.lock().await = Some(result.input);

        let mut result2 = docker.stats(
            &Self::container_name(self.id),
            Some(StatsOptions {
                stream: true,
                one_shot: false,
            }),
        );

        let cpu_max = self.cpu.map(|cpu| cpu * 100f32).unwrap_or(-1f32);
        let mem_max = self
            .ram
            .map(|mem| ((mem as f32) / 1024f32) / 1024f32)
            .unwrap_or(-1f32);
        drop(buffer);

        let server_id = self.id;
        let mut output = result.output;
        let sender = server_data.sender.deref().clone();
        let stats_sender = server_data.stats_sender.deref().clone();
        let buffer = server_data.buffer.clone();
        tokio::task::spawn(async move {
            let mut next = Instant::now().add(Duration::from_millis(500));
            loop {
                tokio::select! {
                    log = output.next() => {
                        match log {
                            Some(Ok(log)) => {
                                let mut buffer = buffer.lock().await;
                                Self::console(&sender, buffer.deref_mut(), log.to_string())
                            },
                            Some(Err(_err)) => return,
                            None => break,
                        }
                    }
                    stat = result2.next() => {
                        if let Some(Ok(stats)) = stat {
                            let _ = stats_sender.send(ChannelWriteData::Stats(<Stats as Into<ServerStats>>::into(stats).with_max(cpu_max, mem_max)));
                        }
                    }
                    _elapsed = tokio::time::sleep_until(next) => {
                        next = Instant::now().add(Duration::from_millis(500));
                        let stats_sender = stats_sender.clone();
                        let docker = docker.clone();
                        let server_data_arc = server_data_arc.clone();
                        tokio::task::spawn(async move {
                            Self::update_status(server_id, &stats_sender, &docker, &server_data_arc).await;
                        });
                    }
                }
            }
            let mut server_data = server_data_arc.lock().await;
            server_data.stopping = false;
            let do_restart = server_data.restart;
            if do_restart {
                server_data.restart = false;
                tokio::task::spawn(Self::queue_start(server_id, state, Duration::from_secs(5)));
            }
            let mut buffer = buffer.lock().await;
            Self::console(&server_data.sender, buffer.deref_mut(), "\n".to_owned());
            Self::log(&server_data.sender, buffer.deref_mut(), "Server stopped!");
            drop(buffer);
            let _ = stats_sender.send(ChannelWriteData::Stats(
                ServerStats::default().with_max(cpu_max, mem_max),
            ));
            if !do_restart {
                tokio::time::sleep(Duration::from_secs(1)).await;
                Self::update_status0(server_id, &docker, &server_data).await;
            }
        });

        Ok(())
    }

    pub(crate) async fn update_tty_size(
        id: u32,
        docker: &Docker,
        server_data: &mut ServerData,
        cols: u32,
        rows: u32,
    ) -> Result<(), ServerStopError> {
        if !Self::state(id, docker, server_data)
            .await
            .map_err(ServerStopError::Docker)?
            .stop()
        {
            return Err(ServerStopError::NotRunning);
        }
        if cols > u16::MAX as u32 || rows > u16::MAX as u32 {
            return Err(ServerStopError::Docker(bollard::errors::Error::IOError {
                err: io::Error::new(ErrorKind::Other, "Integer overflow"),
            }));
        }
        server_data.size = (cols, rows);
        docker
            .resize_container_tty(
                &Self::container_name(id),
                ResizeContainerTtyOptions {
                    width: cols as u16,
                    height: rows as u16,
                },
            )
            .await
            .map_err(ServerStopError::Docker)?;
        Ok(())
    }

    pub(crate) async fn stop(
        &self,
        docker: &Docker,
        server_data: &mut ServerData,
    ) -> Result<(), ServerStopError> {
        if !Self::state(self.id, docker, server_data)
            .await
            .map_err(ServerStopError::Docker)?
            .stop()
        {
            return Err(ServerStopError::NotRunning);
        }
        server_data.stopping = true;
        Self::update_status0(self.id, docker, server_data).await;
        docker
            .stop_container(&Self::container_name(self.id), None)
            .await
            .map_err(ServerStopError::Docker)?;
        Ok(())
    }

    pub(crate) async fn kill(
        &self,
        docker: &Docker,
        server_data: &mut ServerData,
    ) -> Result<(), crate::server::ServerStopError> {
        if !Self::state(self.id, docker, server_data)
            .await
            .map_err(ServerStopError::Docker)?
            .kill()
        {
            return Err(ServerStopError::NotRunning);
        }
        docker
            .kill_container::<String>(&Self::container_name(self.id), None)
            .await
            .map_err(ServerStopError::Docker)?;
        Ok(())
    }

    pub(crate) async fn restart(
        &self,
        docker: &Docker,
        server_data: &mut ServerData,
    ) -> Result<(), crate::server::ServerStopError> {
        if !Self::state(self.id, docker, server_data)
            .await
            .map_err(ServerStopError::Docker)?
            .stop()
        {
            return Err(ServerStopError::NotRunning);
        }
        server_data.restart = true;
        self.stop(docker, server_data).await
    }

    pub(crate) async fn queue_start(server_id: u32, state: DaemonState, duration: Duration) {
        if let Err(err) = Self::queue_start0(server_id, state, duration).await {
            warn!("Failed to queue start: {:?}", err);
        }
    }

    async fn queue_start0(
        server_id: u32,
        state: DaemonState,
        duration: Duration,
    ) -> Result<(), ServerStartError> {
        let (server, server_data) = utils::server(state.conn.clone(), &state.servers, server_id)
            .await
            .map_err(ServerStartError::SQL)?;
        let mut server_data_locked = server_data.lock().await;
        tokio::time::sleep(duration).await;
        server
            .start_pinned(
                &state.config,
                state.docker.clone(),
                server_data_locked.deref_mut(),
                server_data.clone(),
                state.clone(),
            )
            .await?;
        Ok(())
    }

    fn start_pinned<'a>(
        &'a self,
        config: &'a DaemonConfig,
        docker: Arc<Docker>,
        server_data: &'a mut ServerData,
        server_data_arc: Arc<Mutex<ServerData>>,
        state: DaemonState,
    ) -> Pin<Box<dyn Future<Output = Result<(), ServerStartError>> + Send + 'a>> {
        Box::pin(self.start(config, docker, server_data, server_data_arc, state))
        // It needs to be pinned to be called recursively
    }

    pub(crate) async fn update_status(
        id: u32,
        stats_sender: &Sender<ChannelWriteData>,
        docker: &Docker,
        server_data_arc: &Arc<Mutex<ServerData>>,
    ) {
        if stats_sender.receiver_count() == 0 {
            return;
        }
        let server_data = server_data_arc.lock().await;
        Self::update_status0(id, docker, &server_data).await;
    }

    pub(crate) async fn update_status0(id: u32, docker: &Docker, server_data: &ServerData) {
        if let Ok(state) = Self::state(id, docker, server_data).await {
            let _ = server_data
                .stats_sender
                .send(ChannelWriteData::Status(state));
        }
    }
}

#[allow(unused)]
#[derive(Debug)]
pub(crate) enum ServerStartError {
    Docker(bollard::errors::Error),
    Storage(io::Error),
    IO(io::Error),
    AlreadyRunning,
    #[allow(clippy::upper_case_acronyms)]
    SQL(diesel::result::Error), // only if queued
}

#[allow(unused)]
#[derive(Debug)]
pub(crate) enum ServerStopError {
    Docker(bollard::errors::Error),
    NotRunning,
}

impl Display for ServerStartError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let debug_self: &dyn Debug = self; // ensure it's not recursively calling itself
        debug_self.fmt(f)
    }
}

impl Error for ServerStartError {}

impl Display for ServerStopError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let debug_self: &dyn Debug = self; // ensure it's not recursively calling itself
        debug_self.fmt(f)
    }
}

impl Error for ServerStopError {}

#[derive(Clone)]
pub(crate) enum ServerState {
    Stopped,

    // Health-check enabled
    Starting,
    Unhealthy,

    // Health-check disabled
    Running,

    Stopping,
    Restarting,
}

#[derive(Clone, Serialize)]
pub(crate) struct ServerStats {
    cpu: f32,
    mem: f32,
    cpu_max: f32,
    mem_max: f32,
}

impl Default for ServerStats {
    fn default() -> Self {
        Self {
            cpu: -1f32,
            mem: -1f32,
            cpu_max: -1f32,
            mem_max: -1f32,
        }
    }
}

impl ServerStats {
    pub fn with_max(self, cpu: f32, mem: f32) -> Self {
        Self {
            cpu: self.cpu,
            mem: self.mem,
            cpu_max: cpu,
            mem_max: mem,
        }
    }
}

impl From<Stats> for ServerStats {
    fn from(value: Stats) -> Self {
        let mut cpu = -1f32;
        let mut mem = -1f32;

        let system_delta = value.cpu_stats.system_cpu_usage.unwrap_or_default() as f64
            - value.precpu_stats.system_cpu_usage.unwrap_or_default() as f64;
        let cpu_delta = value.cpu_stats.cpu_usage.total_usage as f64
            - value.precpu_stats.cpu_usage.total_usage as f64;

        if value.cpu_stats.online_cpus.is_some() && system_delta > 0f64 && cpu_delta > 0f64 {
            cpu = (((cpu_delta / system_delta)
                * value.cpu_stats.online_cpus.unwrap_or_default() as f64
                * 10000f64)
                .round()
                / 100f64) as f32;
        }

        if let Some(usage) = value.memory_stats.usage {
            let mut usage = usage as f64;
            usage /= 1024f64;
            usage /= 1024f64;
            usage *= 100f64;
            usage = usage.round();
            usage /= 100f64;
            if usage <= f32::MAX as f64 {
                mem = usage as f32;
            }
        }
        Self {
            cpu,
            mem,
            ..Self::default()
        }
    }
}

const STATS_MAGIC: &str = "STATS!";

#[cfg(feature = "websocket")]
impl ServerStats {
    pub fn as_websocket_message(&self) -> Message {
        Message::Text(
            format!(
                "{}{}",
                STATS_MAGIC.to_owned(),
                serde_json::to_string(self).unwrap()
            )
            .to_owned(),
        )
    }
}

impl ServerState {
    pub(crate) fn start(&self) -> bool {
        matches!(self, ServerState::Stopped)
    }

    pub(crate) fn stop(&self) -> bool {
        !matches!(
            self,
            ServerState::Stopped | ServerState::Stopping | ServerState::Restarting
        )
    }

    pub(crate) fn kill(&self) -> bool {
        !matches!(self, ServerState::Stopped)
    }

    pub(crate) fn kill_show(&self) -> bool {
        matches!(self, ServerState::Stopping | ServerState::Restarting)
    }
}

impl Display for ServerState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ServerState::Stopped => "Stopped",
            ServerState::Starting => "Starting",
            ServerState::Unhealthy => "Unhealthy",
            ServerState::Running => "Running",
            ServerState::Stopping => "Stopping",
            ServerState::Restarting => "Restarting",
        })
    }
}

#[derive(Serialize)]
struct ServerStateSerialize {
    label: String,
    start: bool,
    stop: bool,
    kill: bool,
}

#[cfg(feature = "websocket")]
impl ServerState {
    pub fn as_websocket_message(&self) -> Message {
        let mut bytes = Vec::with_capacity(1);
        let mut info = 0u8;
        if self.start() {
            info |= 1 << 0 // set bit 0 (little endian) to 1
        }
        if self.stop() {
            info |= 1 << 1 // set bit 1 (little endian) to 1
        }
        if self.kill_show() {
            info |= 1 << 2 // set bit 2 (little endian) to 1
        }
        bytes.push(info);
        bytes.extend(self.to_string().into_bytes());
        Message::Text(String::from_utf8(bytes).unwrap())
    }
}

impl Serialize for ServerState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        ServerStateSerialize {
            label: self.to_string(),
            start: self.start(),
            stop: self.stop(),
            kill: self.kill(),
        }
        .serialize(serializer)
    }
}
