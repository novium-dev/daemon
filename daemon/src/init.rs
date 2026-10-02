// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::DaemonConfig;
use crate::server::FetchedServer;
use crate::server::watcher::attach;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageError, StorageLockData, storage_read};
use crate::{Daemon, DaemonExtra, WithUserKey, cgroups, ctlsock, schedule, sql, storage};
use bollard::models::{Ipam, IpamConfig, NetworkCreateRequest};
use bollard::query_parameters::{
    InspectNetworkOptions, KillContainerOptions, ListContainersOptions,
};
use core::fmt::Debug;
use core::mem;
use core::time::Duration;
use daemon_common::key::{DaemonKey, DaemonRootKey};
use daemon_common::service::files::task::StorageTask;
use daemon_common::service::server::task::ServerTask;
use ed25519_dalek::SigningKey;
use futures_util::{FutureExt as _, TryFutureExt as _};
use libc::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use sqlx::mysql::MySqlPoolOptions;
use sqlx::query;
use std::collections::HashMap;
use std::io::{self, ErrorKind};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::fs;
use tokio::sync::Mutex;
use tokio::sync::broadcast::Sender;
use tokio::task::{JoinError, JoinSet};
use tracing::{debug, error, info, warn};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum DaemonInitError {
    #[error("init math error")]
    Math,
    #[error("init join error: {0}")]
    Join(#[from] JoinError),
    #[error("init io error: {0}")]
    Io(#[from] io::Error),
    #[error("init sql error: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("init docker error: {0}")]
    Docker(#[from] bollard::errors::Error),
    #[error("init cgroup error: {0}")]
    Cgroup(#[from] cgroups_rs::fs::error::Error),
    #[error("init cgroup error: Cgroup Controller `{0}` not available")]
    CgroupControllerMissing(cgroups_rs::fs::Controllers),
}

impl Daemon<()> {
    pub async fn init(
        config: DaemonConfig,
        root: Arc<DaemonRootKey>,
        key: Arc<DaemonKey>,
        runtime_dir: PathBuf,
    ) -> Result<Self, DaemonInitError> {
        Self::init_internal(config, root, key, runtime_dir, ()).await
    }
}

impl Daemon<WithUserKey> {
    pub async fn init_embedded(
        config: DaemonConfig,
        root: Arc<DaemonRootKey>,
        key: Arc<DaemonKey>,
        runtime_dir: PathBuf,
        mut base_url: Url,
        user_key: SigningKey,
    ) -> Result<Self, DaemonInitError> {
        crate::common::utils::base_url(&mut base_url);
        Self::init_internal(
            config,
            root,
            key,
            runtime_dir,
            WithUserKey { base_url, user_key },
        )
        .await
    }
}

impl<T: DaemonExtra> Daemon<T> {
    async fn init_internal(
        config: DaemonConfig,
        root: Arc<DaemonRootKey>,
        key: Arc<DaemonKey>,
        runtime_dir: PathBuf,
        extra: T,
    ) -> Result<Self, DaemonInitError> {
        #[cfg(target_os = "macos")]
        {
            info!("==========================================================");
            info!("THIS DAEMON IS RUNNING ON MACOS");
            info!("THE DAEMON MIGHT BE VULNERABLE TO TOCTOU ATTACKS!");
            info!("This can expose private information as well as give");
            info!("an attacker full control of your system.");
            info!("As a reminder: This software is provided AS-IS without");
            info!("any warranty. Continue at your own risk.");
            info!("==========================================================");
            info!("Deamon startup will be delayed by 10 sec");
            //tokio::time::sleep(Duration::from_secs(10)).await;
        };

        if !fs::try_exists(&runtime_dir).await? {
            fs::create_dir_all(&runtime_dir).await?;
        }

        let servers = HashMap::new();

        let docker = if cfg!(target_os = "linux") {
            info!("Connecting to Podman/Docker...");
            bollard::Docker::connect_with_podman_defaults()
                .inspect_err(|err| error!("Failed to connect to podman: {err:?}"))?
        } else {
            info!("Connecting to Podman/Docker...");
            bollard::Docker::connect_with_local_defaults()
                .inspect_err(|err| error!("Failed to connect to docker: {err:?}"))?
        };

        match docker
            .inspect_network(
                crate::common::NETWORK_DEFAULT,
                None::<InspectNetworkOptions>,
            )
            .await
        {
            Ok(_) => {}
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {
                let _ = docker
                    .create_network(NetworkCreateRequest {
                        name: crate::common::NETWORK_DEFAULT.to_owned(),
                        options: Some(HashMap::from([
                            (
                                "com.docker.network.bridge.enable_icc".to_owned(),
                                false.to_string(),
                            ),
                            (
                                "com.docker.network.bridge.enable_ip_masquerade".to_owned(),
                                false.to_string(),
                            ),
                        ])),
                        internal: Some(false),
                        enable_ipv6: Some(true),
                        ipam: Some(Ipam {
                            config: Some(vec![IpamConfig {
                                subnet: Some(crate::common::BRIDGE_DEFAULT_IPV6_PREFIX.to_owned()),
                                ..IpamConfig::default()
                            }]),
                            ..Ipam::default()
                        }),
                        ..NetworkCreateRequest::default()
                    })
                    .await
                    .inspect_err(|err| error!("Failed to create docker network: {err:?}"));
            }
            Err(err) => {
                error!("Failed to inspect docker network: {err:?}");
                return Err(DaemonInitError::Docker(err));
            }
        }
        match docker
            .inspect_network(
                crate::common::NETWORK_INTERNET,
                None::<InspectNetworkOptions>,
            )
            .await
        {
            Ok(_) => {}
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {
                let _ = docker
                    .create_network(NetworkCreateRequest {
                        name: crate::common::NETWORK_INTERNET.to_owned(),
                        options: Some(HashMap::from([
                            (
                                "com.docker.network.bridge.enable_icc".to_owned(),
                                false.to_string(),
                            ),
                            (
                                "com.docker.network.bridge.enable_ip_masquerade".to_owned(),
                                true.to_string(),
                            ),
                        ])),
                        internal: Some(false),
                        enable_ipv6: Some(true),
                        ipam: Some(Ipam {
                            config: Some(vec![IpamConfig {
                                subnet: Some(crate::common::BRIDGE_INTERNET_IPV6_PREFIX.to_owned()),
                                ..IpamConfig::default()
                            }]),
                            ..Ipam::default()
                        }),
                        ..NetworkCreateRequest::default()
                    })
                    .await
                    .inspect_err(|err| {
                        error!("Failed to create docker network (internet): {err:?}");
                    });
            }
            Err(err) => {
                error!("Failed to inspect docker network (internet): {err:?}");
                return Err(DaemonInitError::Docker(err));
            }
        }

        info!("Establishing Database Connection...");
        let conn = {
            loop {
                match MySqlPoolOptions::new()
                    .max_connections(5)
                    .connect(&config.database)
                    .await
                {
                    Ok(conn) => break conn,
                    Err(sqlx::Error::Io(io))
                        if io.kind() == ErrorKind::ConnectionRefused
                            || io.kind() == ErrorKind::NotFound => {}
                    Err(err) => {
                        error!("Unable to establish database connection: {:?}", err);
                        return Err(DaemonInitError::Sql(err));
                    }
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        };

        info!("Initializing CGroups...");

        let (cgroup_system, cgroup_server) = cgroups::init(&config).await?;

        let config = Arc::new(config);
        let daemon = Self {
            config: Arc::clone(&config),
            servers: Arc::new(Mutex::new(servers)),
            storages: Arc::default(),
            shutdown_channel: Arc::new(Sender::new(1)),
            server_task: Arc::default(),
            server_task_cancellable: Arc::default(),
            shutdown_task: Arc::default(),
            schedule_task: Arc::default(),
            schedules: Arc::default(),
            docker: Arc::new(docker),
            cgroup_lock: Arc::default(),
            cgroup_system: Arc::new(cgroup_system),
            cgroup_server: Arc::new(cgroup_server),
            runtime_dir,
            started: Instant::now(),
            conn: Arc::new(conn),
            client: Arc::default(),
            extra: Arc::new(extra),

            root,
            key,
        };
        sql::init(&daemon.conn)
            .await
            .inspect_err(|err| error!("Failed to initialize sql: {err:?}"))?;

        daemon.reattach_container().await;

        info!("Initializing CGroups...");

        daemon.sync_cgroups::<DaemonInitError>().await?;

        info!("Running schedules...");

        schedule::init(&daemon)
            .await
            .inspect_err(|err| error!("Failed to run schedules: {:?}", err))?;

        daemon.server_task_cancellable.lock().await.spawn(
            ctlsock::run(daemon.clone())
                .inspect_err(|err| warn!("Failed to run control socket: {err:?}"))
                .map(|_result| ()),
        );

        storage::worker::run(daemon.clone()).await;

        Ok(daemon)
    }

    pub async fn serve_all(&self) -> Result<(), anyhow::Error> {
        self.serve_http().await?;
        self.serve_ssh().await?;
        Ok(())
    }

    pub async fn shutdown(&self) {
        info!("Shutting daemon down...");
        let _ = self.shutdown_channel.send(());
        debug!("Sent shutdown signal");
        self.server_task_cancellable.lock().await.shutdown().await;
        debug!("Cancelled server tasks... (if possible)");
        mem::take(&mut *self.server_task.lock().await)
            .join_all()
            .await;
        debug!("Server tasks down");
        self.schedule_task.lock().await.shutdown().await;
        debug!("Stopped schedules");

        if self.config.shutdown_stop_servers {
            let servers = self.servers.lock().await;
            let mut set = JoinSet::new();
            for (server_id, server) in &*servers {
                let id = *server_id;
                let server = Arc::clone(server);
                let docker = Arc::clone(&self.docker);
                set.spawn(async move {
                    let _ = FetchedServer {
                        id,
                        ..FetchedServer::default()
                    }
                    .stop(&docker, &server, &server.lock.read(ServerTask::Stop).await)
                    .await;
                });
            }
            drop(servers);
            set.join_all().await;
            debug!("Stopped servers");
        }

        info!("Daemon shutdown complete");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    pub fn hook_signals(&self) {
        let daemon = self.clone();
        let mut signal = Signals::new([SIGINT, SIGTERM]).expect("Unable to hook signals");
        tokio::task::spawn_blocking(move || {
            if signal.forever().next().is_some() {
                Arc::clone(&daemon.shutdown_task)
                    .blocking_lock_owned()
                    .spawn(async move { daemon.shutdown().await });
            }
        });
    }

    pub async fn await_shutdown(&self) {
        let mut recv = self.shutdown_channel.subscribe();
        while recv.recv().await.is_err() {}
        mem::take(&mut *self.shutdown_task.lock().await)
            .join_all()
            .await;
    }

    #[expect(
        clippy::significant_drop_tightening,
        reason = "only run once on startup, its better to keep the lock for all loop iterations"
    )]
    async fn reattach_container(&self) {
        if let Ok(containers) = self
            .docker
            .list_containers(None::<ListContainersOptions>)
            .await
            .inspect_err(|err| {
                warn!(
                    "Failed to scan for running containers... (assuming there are none): {:?}",
                    err
                );
            })
        {
            let mut servers = self.servers.lock().await;

            let mut restored = 0i32;
            for container in containers {
                let names = container.names.unwrap_or_default();
                let mut id = None::<u32>;
                for mut name in names.iter().map(String::as_str) {
                    if let Some(suffix) = name.strip_prefix('/') {
                        name = suffix;
                    }
                    if let Some(suffix) = name.strip_prefix(crate::common::CONTAINER_PREFIX)
                        && let Ok(container_id) = suffix.parse()
                    {
                        id = Some(container_id);
                        break;
                    }
                }
                if let Some(id) = id {
                    let server_data = Arc::clone(servers.entry(id).or_default());

                    match query!(
                        "SELECT `cpu`, `ram`, `swap` FROM `servers` WHERE `id` = ?",
                        id
                    )
                    .fetch_optional(&*self.conn)
                    .await
                    {
                        Ok(Some(row)) => {
                            match storage_read::<StorageError>(
                                &self.config,
                                &*self.conn,
                                &self.storages,
                                StorageIdentifier::Primary(id),
                                StorageTask::ServerRunning,
                            )
                            .await
                            {
                                Ok(StorageLockData {
                                    lock: storage_lock,
                                    value: storage_value,
                                    stats_lock: storage_stats_lock,
                                    ..
                                }) => {
                                    if let Err(err) = attach(
                                        self,
                                        id,
                                        row.cpu,
                                        row.ram,
                                        row.swap,
                                        Arc::clone(&server_data),
                                        storage_lock,
                                        storage_value,
                                        storage_stats_lock,
                                    )
                                    .await
                                    {
                                        warn!("Failed to reattach server {id}: {err:?}");
                                    } else {
                                        restored = restored.saturating_add(1);
                                    }
                                }
                                Err(err) => {
                                    warn!("Failed to read lock storage data: {id}: {err:?}");
                                }
                            }
                        }
                        Ok(None) => {
                            warn!(
                                "Found server that is missing in database: {id}. Killing silently..."
                            );
                            let _ = self
                                .docker
                                .kill_container(
                                    &FetchedServer::container_name(id),
                                    None::<KillContainerOptions>,
                                )
                                .await;
                        }
                        Err(err) => warn!("Unable to select server for reattach {id}: {err:?}"),
                    }
                }
            }

            if restored > 0i32 {
                info!("Successfully reattached to {} container", restored);
            }
        }
    }
}
