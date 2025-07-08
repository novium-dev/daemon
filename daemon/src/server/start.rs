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
use crate::server::FetchedServer;
use crate::server::watcher::attach;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageError, StorageLockData, storage_read};
use crate::sync::Mutex;
use crate::utils::from_mib;
use crate::{Daemon, DaemonExtra, ServerData, utils};
use bollard::Docker;
use bollard::models::{
    ContainerCreateBody, EndpointSettings, HostConfig, NetworkingConfig, PortBinding,
};
use bollard::query_parameters::{CreateContainerOptions, StartContainerOptions};
use cap_async_std::ambient_authority;
use cap_async_std::fs::{Dir, OpenOptions};
use core::fmt::{Debug, Display, Formatter};
use core::future::Future;
use core::pin::Pin;
use core::time::Duration;
use daemon_common::service::error_storage_patch_failed::FromStoragePatchFailed;
use daemon_common::service::error_storage_quota_reached::FromStorageQuotaReached;
use num_traits::cast::FromPrimitive as _;
use ringbuffer::RingBuffer as _;
use std::collections::HashMap;
use std::io;
use std::io::ErrorKind;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::OwnedRwLockReadGuard;
use tracing::warn;

#[derive(Debug, thiserror::Error)]
pub enum ServerStartError {
    Docker(#[from] bollard::errors::Error),
    Storage(#[from] StorageError),
    IO(#[from] io::Error),
    PatchFailed,
    QuotaReached,
    AlreadyRunning,
    Sql(#[from] sqlx::Error), // only if queued
    NotFound,
    MathError,
}

impl FromStoragePatchFailed for ServerStartError {
    fn from() -> Self
    where
        Self: Sized,
    {
        Self::PatchFailed
    }
}

impl FromStorageQuotaReached for ServerStartError {
    fn from() -> Self
    where
        Self: Sized,
    {
        Self::QuotaReached
    }
}

impl Display for ServerStartError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> core::fmt::Result {
        let debug_self: &dyn Debug = self; // ensure it's not recursively calling itself
        debug_self.fmt(formatter)
    }
}

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
        }: StorageLockData<OwnedRwLockReadGuard<()>>,
        server_data_arc: Arc<Mutex<ServerData>>,
        daemon: &Daemon<impl DaemonExtra>,
    ) -> Result<(), ServerStartError> {
        if !Self::state(self.id, self.locks.clone(), &docker, Some(server_data))
            .await
            .map_err(ServerStartError::Docker)?
            .start()
        {
            return Err(ServerStartError::AlreadyRunning); // Containers are created with delete-on-exit.
        }

        server_data.stop_command.clone_from(&self.stop_command);

        server_data.stopping = false;
        server_data.restart = false;

        let buffer = Arc::clone(&server_data.buffer);
        let mut buffer = buffer.lock().await;
        buffer.clear();
        Self::log(&server_data.sender, &mut buffer, "Preparing Server...");

        let image = self
            .clone()
            .prepare_image(daemon, &docker, &mut buffer, server_data)
            .await?;

        let storage_path_buf = PathBuf::from(&storage_path);
        let storage = Dir::open_ambient_dir(&storage_path_buf, ambient_authority())
            .await
            .map_err(ServerStartError::IO)?;
        let storage2 = storage.clone();
        let _ = storage2.remove_dir_all(".tmp").await;
        if let Err(err) = tokio::task::spawn_blocking(move || storage2.create_dir(".tmp"))
            .await
            .expect("Temporary directory creation task panicked")
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
            let file = tokio::fs::File::from_std(std::fs::File::from(fd));
            file_write.apply::<ServerStartError>(file).await?;
        }

        let mut binds = Vec::with_capacity(1);

        for mount in &daemon.config.mounts {
            binds.push(format!("{}:{}:{}", mount.src, mount.dest, mount.mode));
        }

        let control_path = tokio::fs::canonicalize("./control.sock")
            .await
            .map_err(ServerStartError::IO)?;

        binds.push(format!("{}:/control.sock", control_path.to_string_lossy()));

        binds.push(format!("{storage_path}:/work"));

        let mut networks = HashMap::with_capacity(1);

        networks.insert(
            if self.internet {
                crate::common::NETWORK_INTERNET
            } else {
                crate::common::NETWORK_DEFAULT
            }
            .to_owned(),
            EndpointSettings::default(),
        );

        for network in &self.networks {
            let ipv4_1 = (network.network_ipv4 >> 8u16) as u8;
            #[expect(clippy::cast_possible_truncation, reason = "intentional truncation")]
            let ipv4_2 = network.network_ipv4 as u8 | network.server_ipv4;

            #[expect(clippy::cast_possible_truncation, reason = "intentional truncation")]
            let ipv6_1 = (network.server_ipv6 >> 12u64) as u16;
            #[expect(clippy::cast_possible_truncation, reason = "intentional truncation")]
            let ipv6_2 = (network.server_ipv6 >> 8u64) as u16;
            #[expect(clippy::cast_possible_truncation, reason = "intentional truncation")]
            let ipv6_3 = (network.server_ipv6 >> 4u64) as u16;
            #[expect(clippy::cast_possible_truncation, reason = "intentional truncation")]
            let ipv6_4 = network.server_ipv6 as u16;
            networks.insert(
                format!("{}{}", crate::common::NETWORK_PREFIX, network.network_id),
                EndpointSettings {
                    ip_address: Some(format!(
                        "{}.{}.{ipv4_1}.{ipv4_2}",
                        crate::common::IPV4_PREFIX[0],
                        crate::common::IPV4_PREFIX[1],
                    )),
                    global_ipv6_address: Some(format!(
                        "{:x}:{:x}:{:x}:{:x}:{ipv6_1:x}:{ipv6_2:x}:{ipv6_3:x}:{ipv6_4:x}",
                        crate::common::IPV6_PREFIX[0],
                        crate::common::IPV6_PREFIX[1],
                        crate::common::IPV6_PREFIX[2],
                        network.network_ipv6,
                    )),
                    ..EndpointSettings::default()
                },
            );
        }

        let mut ports = HashMap::new();
        for port in &self.ports {
            let key = format!("{}/{}", port.internal, port.protocol);
            ports.entry(key).or_insert_with(Vec::new).push(PortBinding {
                host_ip: Some(port.external_ip.clone()),
                host_port: Some(port.external.to_string()),
            });
        }

        Self::log(&server_data.sender, &mut buffer, "Creating Environment...");

        let mut env: HashMap<String, String> =
            HashMap::from_iter([("TERM".to_owned(), "xterm-256color".to_owned())]);
        env.extend(self.environment.clone());
        let env = env
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();

        let memory = self
            .ram
            .map(from_mib)
            .map(|ram| i64::try_from(ram).unwrap_or(i64::MAX));

        docker
            .create_container(
                Some(CreateContainerOptions {
                    name: Some(Self::container_name(self.id)),
                    ..CreateContainerOptions::default()
                }),
                ContainerCreateBody {
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
                        endpoints_config: Some(networks),
                    }),

                    stop_timeout: Some(i64::from(self.stop_timeout)),

                    #[expect(
                        clippy::zero_sized_map_values,
                        reason = "needed because of struct value"
                    )]
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

                        port_bindings: Some(
                            ports
                                .into_iter()
                                .map(|(key, ports)| (key, Some(ports)))
                                .collect(),
                        ),

                        extra_hosts: Some(
                            crate::common::HOSTNAME
                                .iter()
                                .map(|hostname| format!("{hostname}:host-gateway"))
                                .collect(),
                        ),

                        cap_add: Some(self.cap_add.iter().cloned().collect()),
                        cap_drop: Some(self.cap_drop.iter().cloned().collect()),

                        cpu_period: self.cpu.map(|_| 100_000i64),
                        cpu_quota: if let Some(cpu) = self.cpu {
                            Some(
                                i64::from_f64((f64::from(cpu) * 100_000.0f64).round())
                                    .ok_or(ServerStartError::MathError)?,
                            )
                        } else {
                            None
                        },
                        memory,
                        memory_swap: memory
                            .map(|memory| {
                                self.swap.map(from_mib).map(|swap| {
                                    i64::try_from(swap)
                                        .unwrap_or(i64::MAX)
                                        .saturating_add(memory)
                                })
                            })
                            .unwrap_or_default(),

                        #[cfg(not(feature = "no_tty_size_please"))]
                        console_size: Some(vec![
                            server_data.size.1 as usize,
                            server_data.size.0 as usize,
                        ]),

                        ..HostConfig::default()
                    }),
                    ..ContainerCreateBody::default()
                },
            )
            .await
            .map_err(ServerStartError::Docker)?;

        Self::log(&server_data.sender, &mut buffer, "Starting Server...");

        docker
            .start_container(
                &Self::container_name(self.id),
                None::<StartContainerOptions>,
            )
            .await
            .map_err(ServerStartError::Docker)?;

        drop(buffer);

        let server_id = self.id;
        attach(
            daemon,
            server_id,
            self.cpu,
            self.ram,
            self.swap,
            server_data,
            server_data_arc,
            storage_lock,
            storage_value,
            storage_stats_lock,
        )
        .await?;

        Ok(())
    }

    pub(crate) async fn queue_start(
        daemon: Daemon<impl DaemonExtra>,
        server_id: u32,
        duration: Duration,
    ) {
        if let Err(err) = Self::queue_start0(daemon, server_id, duration).await {
            warn!("Failed to queue start: {:?}", err);
        }
    }

    #[inline(always)]
    #[expect(
        clippy::significant_drop_tightening,
        reason = "lock the server data first"
    )]
    async fn queue_start0(
        daemon: Daemon<impl DaemonExtra>,
        server_id: u32,
        duration: Duration,
    ) -> Result<(), ServerStartError> {
        let Some((server, server_data)) = utils::server(&daemon.conn, &daemon.servers, server_id)
            .await
            .map_err(ServerStartError::Sql)?
        else {
            return Err(ServerStartError::NotFound);
        };
        let mut server_data_locked = server_data.lock().await;
        tokio::time::sleep(duration).await;
        let storage = storage_read::<StorageError>(
            &daemon.config,
            &*daemon.conn,
            &daemon.storages,
            StorageIdentifier::Primary(server_id),
        )
        .await
        .map_err(ServerStartError::Storage)?;
        server
            .start_pinned(
                Arc::clone(&daemon.docker),
                &mut server_data_locked,
                Arc::clone(&server_data),
                storage,
                &daemon,
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
        storage: StorageLockData<OwnedRwLockReadGuard<()>>,
        daemon: &'a Daemon<impl DaemonExtra>,
    ) -> Pin<Box<dyn Future<Output = Result<(), ServerStartError>> + Send + 'a>> {
        Box::pin(self.start(docker, server_data, storage, server_data_arc, daemon))
        // It needs to be pinned to be called recursively
    }
}
