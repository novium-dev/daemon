use crate::server::watcher::attach;
use crate::server::FetchedServer;
use crate::storage::sync::{storage_read, StorageError, StorageLockData};
use crate::storage::StorageIdentifier;
use crate::utils::from_mib;
use crate::{utils, DaemonState, ServerData};
use bollard::container::{Config, CreateContainerOptions, NetworkingConfig};
use bollard::models::{EndpointSettings, HostConfig, PortBinding};
use bollard::Docker;
use cap_async_std::ambient_authority;
use cap_async_std::fs::{Dir, OpenOptions};
use ringbuffer::RingBuffer;
use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::future::Future;
use std::io;
use std::io::ErrorKind;
use std::ops::{Deref, DerefMut};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, OwnedRwLockReadGuard};
use tracing::warn;

#[allow(unused)]
#[derive(Debug)]
pub(crate) enum ServerStartError {
    Docker(bollard::errors::Error),
    Storage(StorageError),
    IO(io::Error),
    AlreadyRunning,
    #[allow(clippy::upper_case_acronyms)]
    SQL(sqlx::Error), // only if queued
    NotFound,
}

impl Display for ServerStartError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let debug_self: &dyn Debug = self; // ensure it's not recursively calling itself
        debug_self.fmt(f)
    }
}

impl Error for ServerStartError {}

impl FetchedServer {
    pub(crate) async fn start(
        &self,
        docker: Arc<Docker>,
        server_data: &mut ServerData,
        StorageLockData {
            path: storage_path,
            lock: storage_lock,
            value: storage_value,
            stats_lock: storage_stats_lock,
            ..
        }: StorageLockData<'_, OwnedRwLockReadGuard<()>>,
        server_data_arc: Arc<Mutex<ServerData>>,
        state: &DaemonState,
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
            .prepare_image(state, &docker, buffer.deref_mut(), server_data)
            .await?;

        let storage_path_buf = PathBuf::from(&storage_path);
        let storage = Dir::open_ambient_dir(&storage_path_buf, ambient_authority())
            .await
            .map_err(ServerStartError::IO)?;
        let storage2 = storage.clone();
        let _ = storage2.remove_dir_all(".tmp").await;
        if let Err(err) = tokio::task::spawn_blocking(move || storage2.create_dir(".tmp"))
            .await
            .unwrap()
        {
            if err.kind() != ErrorKind::AlreadyExists {
                return Err(ServerStartError::IO(err));
            }
        }

        for (file_path, file_write) in &self.files_write {
            let file = storage
                .open_with(
                    file_path,
                    OpenOptions::new().create(true).write(true).read(true),
                )
                .await
                .map_err(ServerStartError::IO)?;
            let fd: OwnedFd = file.into();
            let mut file = tokio::fs::File::from_std(std::fs::File::from(fd));
            file_write
                .apply(&mut file)
                .await
                .map_err(ServerStartError::IO)?;
        }

        let mut binds = Vec::with_capacity(1);

        for mount in &state.config.mounts {
            binds.push(format!("{}:{}:{}", mount.src, mount.dest, mount.mode))
        }

        let control_path = tokio::fs::canonicalize("./control.sock")
            .await
            .map_err(ServerStartError::IO)?;

        binds.push(format!("{}:/control.sock", control_path.to_string_lossy()));

        binds.push(format!("{}:/work", storage_path));

        let mut networks = HashMap::with_capacity(1);

        networks.insert(
            if self.internet {
                crate::common::NETWORK_INTERNET
            } else {
                crate::common::NETWORK_DEFAULT
            }
            .to_string(),
            EndpointSettings::default(),
        );

        for network in &self.networks {
            let ipv4_1 = (network.network_ipv4 >> 8) as u8;
            let ipv4_2 = network.network_ipv4 as u8 | network.server_ipv4;

            let ipv6_1 = (network.server_ipv6 >> 12) as u16;
            let ipv6_2 = (network.server_ipv6 >> 8) as u16;
            let ipv6_3 = (network.server_ipv6 >> 4) as u16;
            let ipv6_4 = network.server_ipv6 as u16;
            networks.insert(
                format!("{}{}", crate::common::NETWORK_PREFIX, network.network_id),
                EndpointSettings {
                    ip_address: Some(format!(
                        "{}.{}.{}.{}",
                        crate::common::IPV4_PREFIX[0],
                        crate::common::IPV4_PREFIX[1],
                        ipv4_1,
                        ipv4_2
                    )),
                    global_ipv6_address: Some(format!(
                        "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
                        crate::common::IPV6_PREFIX[0],
                        crate::common::IPV6_PREFIX[1],
                        crate::common::IPV6_PREFIX[2],
                        network.network_ipv6,
                        ipv6_1,
                        ipv6_2,
                        ipv6_3,
                        ipv6_4
                    )),
                    ..EndpointSettings::default()
                },
            );
        }

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

        let mut env: HashMap<String, String> =
            HashMap::from_iter([("TERM".to_owned(), "xterm-256color".to_owned())]);
        env.extend(self.environment.clone());
        let env = env
            .into_iter()
            .map(|(key, value)| format!("{}={}", key, value))
            .collect();

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

                    user: Some(format!(
                        "{}:{}",
                        self.uid.unwrap_or(crate::common::CONTAINER_DEFAULT_UID),
                        self.gid.unwrap_or(crate::common::CONTAINER_DEFAULT_GID)
                    )),

                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    attach_stdin: Some(true),

                    env: Some(env),

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
                        readonly_rootfs: Some(true),
                        auto_remove: Some(true),

                        binds: Some(binds),

                        port_bindings: Some(ports),

                        extra_hosts: Some(vec![format!(
                            "{}:host-gateway",
                            crate::common::HOSTNAME
                        )]),

                        cap_add: Some(self.cap_add.iter().cloned().collect()),
                        cap_drop: Some(self.cap_drop.iter().cloned().collect()),

                        cpu_period: self.cpu.map(|_| 100000),
                        cpu_quota: self.cpu.map(|cpu| (100000.0 * cpu) as i64),
                        memory: self.ram.map(utils::from_mib).map(|ram| ram as i64),
                        memory_swap: self.ram.map(|data| {
                            if let Some(swap) = self.swap {
                                from_mib(data + swap) as i64
                            } else {
                                -1
                            }
                        }),

                        console_size: Some(vec![
                            server_data.size.0 as i32,
                            server_data.size.1 as i32,
                        ]),

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

        let cpu_max = self.cpu.map(|cpu| cpu * 100f32).unwrap_or(-1f32);
        let ram_max = self.ram.map(|ram| ram as i64).unwrap_or(-1);
        let swap_max = self.swap.map(|ram| ram as i64).unwrap_or(-1);
        drop(buffer);

        let server_id = self.id;
        attach(
            server_id,
            state,
            cpu_max,
            ram_max,
            swap_max,
            server_data,
            server_data_arc,
            storage_lock,
            storage_value,
            storage_stats_lock,
        )
        .await?;

        Ok(())
    }

    pub(crate) async fn queue_start(server_id: u32, state: DaemonState, duration: Duration) {
        if let Err(err) = Self::queue_start0(server_id, state, duration).await {
            warn!("Failed to queue start: {:?}", err);
        }
    }

    #[inline(always)]
    async fn queue_start0(
        server_id: u32,
        state: DaemonState,
        duration: Duration,
    ) -> Result<(), ServerStartError> {
        let Some((server, server_data)) = utils::server(&state.conn, &state.servers, server_id)
            .await
            .map_err(ServerStartError::SQL)?
        else {
            return Err(ServerStartError::NotFound);
        };
        let mut server_data_locked = server_data.lock().await;
        tokio::time::sleep(duration).await;
        let storage = storage_read(
            &state.config,
            state.conn.deref(),
            &state.storages,
            StorageIdentifier::Primary(server_id),
            false,
        )
        .await
        .map_err(ServerStartError::Storage)?;
        server
            .start_pinned(
                state.docker.clone(),
                server_data_locked.deref_mut(),
                server_data.clone(),
                storage,
                &state,
            )
            .await?;
        Ok(())
    }

    #[inline(always)]
    fn start_pinned<'a>(
        &'a self,
        docker: Arc<Docker>,
        server_data: &'a mut ServerData,
        server_data_arc: Arc<Mutex<ServerData>>,
        storage: StorageLockData<'a, OwnedRwLockReadGuard<()>>,
        state: &'a DaemonState,
    ) -> Pin<Box<dyn Future<Output = Result<(), ServerStartError>> + Send + 'a>> {
        Box::pin(self.start(docker, server_data, storage, server_data_arc, state))
        // It needs to be pinned to be called recursively
    }
}
