use crate::cgroup::clamp_cpu_limit;
// SPDX-License-Identifier: AGPL-3.0-only
use crate::server::FetchedServer;
use crate::server::watcher::attach;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageError, StorageLockData, storage_read};
use crate::sync::task::{AnyTaskRwLockWriteGuard, TaskOwnedRwLockReadGuard};
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
use core::sync::atomic::Ordering;
use core::time::Duration;
use daemon_common::service::error_storage_patch_failed::FromStoragePatchFailed;
use daemon_common::service::error_storage_quota_reached::FromStorageQuotaReached;
use daemon_common::service::files::task::StorageTask;
use daemon_common::service::server::task::ServerTask;
use ringbuffer::RingBuffer as _;
use std::collections::HashMap;
use std::io;
use std::io::ErrorKind;
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::warn;

#[derive(Debug, thiserror::Error)]
pub enum ServerStartError {
    Docker(#[from] bollard::errors::Error),
    Storage(#[from] StorageError),
    IO(#[from] io::Error),
    PatchFailed,
    QuotaReached,
    AlreadyRunning,
    DBus(#[from] zbus::Error),
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
        server_data: Arc<ServerData>,
        lock: &mut impl AnyTaskRwLockWriteGuard<ServerTask, ()>,
        StorageLockData {
            path: storage_path,
            lock: storage_lock,
            value: storage_value,
            stats_lock: storage_stats_lock,
            ..
        }: StorageLockData<TaskOwnedRwLockReadGuard<StorageTask, ()>>,
        daemon: &Daemon<impl DaemonExtra>,
    ) -> Result<(), ServerStartError> {
        if !Self::container_state(self.id, &self.locks, &docker, &server_data)
            .await
            .map_err(ServerStartError::Docker)?
            .start()
        {
            return Err(ServerStartError::AlreadyRunning); // Containers are created with delete-on-exit.
        }

        let _ = &mut *lock;

        server_data
            .stop_command
            .lock()
            .await
            .clone_from(&self.stop_command);

        server_data.stopping.store(false, Ordering::Relaxed);
        server_data.restart.store(false, Ordering::Relaxed);

        Self::update_status(self.id, &self.locks, &docker, &server_data).await;

        server_data.buffer.lock().await.clear();
        Self::log(
            &server_data.sender,
            &server_data.buffer,
            "Preparing Server...",
        )
        .await;

        daemon.sync_cgroups::<ServerStartError>().await?;

        let image = self
            .clone()
            .prepare_image(daemon, &docker, &server_data)
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
            && err.kind() != ErrorKind::AlreadyExists
        {
            return Err(ServerStartError::IO(err));
        }

        for (file_path, file_write) in &self.files_write {
            if let Some(parent) = Path::new(file_path).parent()
                && parent != Path::new("")
            {
                storage
                    .create_dir_all(parent)
                    .map_err(ServerStartError::IO)?;
            }

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

        let control_path = tokio::fs::canonicalize(daemon.runtime_dir.join("control.sock"))
            .await
            .map_err(ServerStartError::IO)?;

        binds.push(Self::bind(&control_path, "/control.sock").map_err(ServerStartError::IO)?);

        binds.push(format!(
            "{storage_path}:{}",
            crate::common::CONTAINER_WORK_DIR
        ));

        let uid = self.uid.unwrap_or(crate::common::CONTAINER_DEFAULT_UID);
        let gid = self.gid.unwrap_or(crate::common::CONTAINER_DEFAULT_GID);

        let machine_id_path = Self::write_machine_id(&daemon.runtime_dir, self.id)
            .await
            .map_err(ServerStartError::IO)?;

        binds.push(
            Self::bind(&machine_id_path, "/etc/machine-id:ro").map_err(ServerStartError::IO)?,
        );

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
            let ipv4_1 = (network.network_ipv4 >> 8u16) & 0xff;
            let ipv4_2 = (network.network_ipv4 & 0xff) as u8 | network.server_ipv4;

            let ipv6_1 = (network.server_ipv6 >> 12u64) & 0xffff;
            let ipv6_2 = (network.server_ipv6 >> 8u64) & 0xffff;
            let ipv6_3 = (network.server_ipv6 >> 4u64) & 0xffff;
            let ipv6_4 = network.server_ipv6 & 0xffff;
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

        Self::log(
            &server_data.sender,
            &server_data.buffer,
            "Creating Environment...",
        )
        .await;

        let mut env: HashMap<String, String> = HashMap::from_iter([
            ("TERM".to_owned(), "xterm-256color".to_owned()),
            ("TZ".to_owned(), utils::host_timezone().to_owned()),
            (
                "HOME".to_owned(),
                crate::common::CONTAINER_WORK_DIR.to_owned(),
            ),
            ("USER".to_owned(), crate::common::CONTAINER_USER.to_owned()),
        ]);
        env.extend(self.environment.clone()); // the server may override the defaults above
        let env = env
            .into_iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect();

        let memory = self
            .ram
            .map(from_mib)
            .map(|ram| i64::try_from(ram).unwrap_or(i64::MAX));

        let cpu = self.cpu.and_then(clamp_cpu_limit::<false>);

        let created = docker
            .create_container(
                Some(CreateContainerOptions {
                    name: Some(Self::container_name(self.id)),
                    ..CreateContainerOptions::default()
                }),
                ContainerCreateBody {
                    tty: Some(true),
                    open_stdin: Some(true),

                    image: Some(image),

                    user: Some(format!("{uid}:{gid}")),

                    working_dir: Some(crate::common::CONTAINER_WORK_DIR.to_owned()),

                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    attach_stdin: Some(true),

                    env: Some(env),

                    networking_config: Some(NetworkingConfig {
                        endpoints_config: Some(networks),
                    }),

                    stop_timeout: Some(i64::from(self.stop_timeout)),

                    exposed_ports: Some(ports.keys().cloned().collect()),

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

                        cpu_period: if let Some((_quota, period)) = cpu {
                            Some(
                                i64::try_from(period)
                                    .map_err(|_err| ServerStartError::MathError)?,
                            )
                        } else {
                            None
                        },
                        cpu_quota: if let Some((quota, _period)) = cpu {
                            Some(i64::try_from(quota).map_err(|_err| ServerStartError::MathError)?)
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

                        console_size: Some(vec![
                            server_data.size.1.load(Ordering::Relaxed) as usize,
                            server_data.size.0.load(Ordering::Relaxed) as usize,
                        ]),

                        #[cfg(target_os = "linux")]
                        cgroup_parent: Some(crate::cgroup::systemd_slice_handle(
                            &self
                                .group
                                .as_deref()
                                .map(std::borrow::Cow::Borrowed)
                                .map_or(
                                    crate::cgroup::CgroupIdentifier::Server,
                                    crate::cgroup::CgroupIdentifier::Group,
                                ),
                        )),
                        ..HostConfig::default()
                    }),
                    ..ContainerCreateBody::default()
                },
            )
            .await;
        if let Err(err) = created {
            Self::log(
                &server_data.sender,
                &server_data.buffer,
                "Container creation failed",
            )
            .await;
            return Err(ServerStartError::Docker(err));
        }

        Self::log(
            &server_data.sender,
            &server_data.buffer,
            "Starting Server...",
        )
        .await;

        let started = docker
            .start_container(
                &Self::container_name(self.id),
                None::<StartContainerOptions>,
            )
            .await;
        if let Err(err) = started {
            Self::log(
                &server_data.sender,
                &server_data.buffer,
                "Container start failed",
            )
            .await;
            return Err(ServerStartError::Docker(err));
        }

        let server_id = self.id;
        attach(
            daemon,
            server_id,
            self.cpu,
            self.ram,
            self.swap,
            server_data,
            storage_lock,
            storage_value,
            storage_stats_lock,
        )
        .await?;

        Ok(())
    }

    /// Formats a bind mount of `source` on the host to `target` in the container.
    ///
    /// Bind mounts are passed to docker as a string, so a path that is not valid utf-8
    /// can not be mounted at all and has to be rejected instead of being mangled.
    fn bind(source: &Path, target: &str) -> Result<String, io::Error> {
        let Some(source) = source.to_str() else {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                format!(
                    "the path {} is not valid utf-8 and can not be mounted",
                    source.display()
                ),
            ));
        };

        Ok(format!("{source}:{target}"))
    }

    /// Generates the `/etc/machine-id` that is mounted into the container and returns its
    /// path on the host.
    ///
    /// The read-only root filesystem leaves the servers without a machine id, while some of
    /// them (e.g. Hytale, which derives the encryption of its tokens from it) require one.
    async fn write_machine_id(runtime_dir: &Path, server_id: u32) -> Result<PathBuf, io::Error> {
        let dir = runtime_dir.join("containers").join(server_id.to_string());
        tokio::fs::create_dir_all(&dir).await?;
        let dir = tokio::fs::canonicalize(&dir).await?;

        let path = dir.join("machine-id");
        tokio::fs::write(&path, format!("{}\n", utils::server_machine_id(server_id))).await?;
        // The container has to be able to read it, no matter which umask the daemon runs with
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).await?;

        Ok(path)
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
        let mut lock = server_data.lock.write(ServerTask::Start).await;
        tokio::time::sleep(duration).await;
        let storage = storage_read::<StorageError>(
            &daemon.config,
            &*daemon.conn,
            &daemon.storages,
            StorageIdentifier::Primary(server_id),
            StorageTask::ServerRunning,
        )
        .await
        .map_err(ServerStartError::Storage)?;
        server
            .start_pinned(
                Arc::clone(&daemon.docker),
                Arc::clone(&server_data),
                &mut lock,
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
        server_data: Arc<ServerData>,
        lock: &'a mut impl AnyTaskRwLockWriteGuard<ServerTask, ()>,
        storage: StorageLockData<TaskOwnedRwLockReadGuard<StorageTask, ()>>,
        daemon: &'a Daemon<impl DaemonExtra>,
    ) -> Pin<Box<dyn Future<Output = Result<(), ServerStartError>> + Send + 'a>> {
        Box::pin(self.start(docker, server_data, lock, storage, daemon))
        // It needs to be pinned to be called recursively
    }
}
