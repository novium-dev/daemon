use crate::config::DaemonConfig;
use crate::server::FetchedServer;
use crate::server::watcher::attach;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageError, StorageLockData, storage_read};
use crate::sync::Mutex;
use crate::{Daemon, ctlsock, schedule, sql, storage};
use bollard::models::{Ipam, IpamConfig, NetworkCreateRequest};
use bollard::query_parameters::{
    InspectNetworkOptions, KillContainerOptions, ListContainersOptions,
};
use daemon_common::key::{DaemonKey, DaemonRootKey};
use ed25519_dalek::SigningKey;
use libc::{SIGINT, SIGTERM};
use reqwest::Client;
use signal_hook::iterator::Signals;
use sqlx::mysql::MySqlPoolOptions;
use sqlx::query;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::Sender;
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};
use url::Url;

#[derive(Debug)]
pub enum DaemonInitError {
    Sql(sqlx::Error),
    Docker(bollard::errors::Error),
}

impl Daemon {
    pub async fn init(
        config: DaemonConfig,
        root: DaemonRootKey,
        key: DaemonKey,
    ) -> Result<Self, DaemonInitError> {
        Self::init_embedded(
            config,
            root,
            key,
            Url::parse("http://localhost:3000/").unwrap(),
            SigningKey::from_bytes(&[0; 32]),
        )
        .await
    }

    pub async fn init_embedded(
        config: DaemonConfig,
        root: DaemonRootKey,
        key: DaemonKey,

        mut base_url: Url,
        user_key: SigningKey,
    ) -> Result<Self, DaemonInitError> {
        crate::common::utils::base_url(&mut base_url);

        let servers = HashMap::new();

        info!("Connecting to Docker...");
        let docker = bollard::Docker::connect_with_local_defaults()
            .inspect_err(|_err| error!("Failed to connect to docker"))
            .unwrap();

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
                docker
                    .create_network(NetworkCreateRequest {
                        name: crate::common::NETWORK_DEFAULT.to_string(),
                        options: Some(HashMap::from([
                            (
                                "com.docker.network.bridge.enable_icc".to_string(),
                                false.to_string(),
                            ),
                            (
                                "com.docker.network.bridge.enable_ip_masquerade".to_string(),
                                false.to_string(),
                            ),
                        ])),
                        internal: Some(false),
                        enable_ipv6: Some(true),
                        ipam: Some(Ipam {
                            config: Some(vec![IpamConfig {
                                subnet: Some(crate::common::BRIDGE_DEFAULT_IPV6_PREFIX.to_string()),
                                ..IpamConfig::default()
                            }]),
                            ..Ipam::default()
                        }),
                        ..NetworkCreateRequest::default()
                    })
                    .await
                    .unwrap();
            }
            Err(err) => {
                error!("Failed to inspect docker network: {:?}", err);
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
                docker
                    .create_network(NetworkCreateRequest {
                        name: crate::common::NETWORK_INTERNET.to_string(),
                        options: Some(HashMap::from([
                            (
                                "com.docker.network.bridge.enable_icc".to_string(),
                                false.to_string(),
                            ),
                            (
                                "com.docker.network.bridge.enable_ip_masquerade".to_string(),
                                true.to_string(),
                            ),
                        ])),
                        internal: Some(false),
                        enable_ipv6: Some(true),
                        ipam: Some(Ipam {
                            config: Some(vec![IpamConfig {
                                subnet: Some(
                                    crate::common::BRIDGE_INTERNET_IPV6_PREFIX.to_string(),
                                ),
                                ..IpamConfig::default()
                            }]),
                            ..Ipam::default()
                        }),
                        ..NetworkCreateRequest::default()
                    })
                    .await
                    .unwrap();
            }
            Err(err) => {
                error!("Failed to inspect docker network (internet): {:?}", err);
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

        let config = Arc::new(config);
        let daemon = Daemon {
            config: config.clone(),
            servers: Arc::new(Mutex::new(servers)),
            storages: Arc::new(Mutex::new(HashMap::new())),
            shutdown_channel: Arc::new(Sender::new(1)),
            server_task: Arc::new(Mutex::new(JoinSet::new())),
            server_task_cancellable: Arc::new(Mutex::new(JoinSet::new())),
            shutdown_task: Arc::new(Mutex::new(JoinSet::new())),
            schedule_task: Arc::new(Mutex::new(JoinSet::new())),
            schedules: Arc::new(Mutex::new(HashMap::new())),
            docker: Arc::new(docker),
            conn: Arc::new(conn),
            client: Arc::new(Client::default()),

            root: Arc::new(root),
            key: Arc::new(key),

            base_url,
            user_key,
        };
        sql::init(&daemon.conn)
            .await
            .inspect_err(|err| error!("Failed to initialize sql: {:?}", err))
            .map_err(DaemonInitError::Sql)?;

        daemon.reattach_container().await;

        info!("Running schedules...");

        schedule::init(&daemon)
            .await
            .inspect_err(|err| error!("Failed to run schedules: {:?}", err))
            .map_err(DaemonInitError::Sql)?;

        daemon
            .server_task_cancellable
            .lock()
            .await
            .spawn(ctlsock::run(daemon.clone()));

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
        mem::take(self.server_task.lock().await.deref_mut())
            .join_all()
            .await;
        debug!("Server tasks down");
        let servers = self.servers.lock().await;
        let mut set = JoinSet::new();
        for (id, server) in servers.deref() {
            let server = server.clone();
            let docker = self.docker.clone();
            let id = *id;
            set.spawn(async move {
                let _ = FetchedServer {
                    id,
                    ..FetchedServer::default()
                }
                .stop(&docker, server.lock_owned().await)
                .await;
            });
        }
        self.schedule_task.lock().await.shutdown().await;
        debug!("Stopped schedules");
        set.join_all().await;
        debug!("Stopped servers");

        info!("Daemon shutdown complete");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    pub fn hook_signals(&self) {
        let daemon = self.clone();
        let mut signal = Signals::new([SIGINT, SIGTERM]).unwrap();
        tokio::task::spawn_blocking(move || {
            if signal.forever().next().is_some() {
                daemon
                    .shutdown_task
                    .clone()
                    .blocking_lock_owned()
                    .spawn(async move { daemon.shutdown().await });
            }
        });
    }

    pub async fn await_shutdown(&self) {
        let mut recv = self.shutdown_channel.subscribe();
        while recv.recv().await.is_err() {}
        mem::take(self.shutdown_task.lock().await.deref_mut())
            .join_all()
            .await;
    }

    async fn reattach_container(&self) {
        if let Ok(containers) = self
            .docker
            .list_containers(None::<ListContainersOptions>)
            .await
            .inspect_err(|err| {
                warn!(
                    "Failed to scan for running containers... (assuming there are none): {:?}",
                    err
                )
            })
        {
            let mut servers = self.servers.lock().await;

            let mut restored = 0i32;
            for container in containers {
                let names = container.names.unwrap_or_default();
                let mut id = None::<u32>;
                for mut name in names {
                    if let Some(suffix) = name.strip_prefix("/") {
                        name = suffix.to_string();
                    }
                    if let Some(suffix) = name.strip_prefix(crate::common::CONTAINER_PREFIX) {
                        if let Ok(container_id) = suffix.parse() {
                            id = Some(container_id);
                            break;
                        }
                    }
                }
                if let Some(id) = id {
                    servers.insert(id, Arc::default());
                    let server_data_arc = servers.get(&id).unwrap().clone();

                    match query!(
                        "SELECT `cpu`, `ram`, `swap` FROM `servers` WHERE `id` = ?",
                        id
                    )
                    .fetch_optional(&*self.conn)
                    .await
                    {
                        Ok(Some(row)) => {
                            let mut server_data = server_data_arc.lock().await;

                            match storage_read::<StorageError>(
                                &self.config,
                                &*self.conn,
                                &self.storages,
                                StorageIdentifier::Primary(id),
                            )
                            .await {
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
                                        &mut server_data,
                                        Arc::clone(&server_data_arc),
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
                                Err(err) => warn!("Failed to read lock storage data: {id}: {err:?}"),
                            }
                        }
                        Ok(None) => {
                            warn!(
                                "Found server that is missing in database: {}. Killing silently...",
                                id
                            );
                            let _ = self
                                .docker
                                .kill_container(
                                    &FetchedServer::container_name(id),
                                    None::<KillContainerOptions>,
                                )
                                .await;
                        },
                        Err(err) => warn!("Unable to select server for reattach {id}: {err:?}"),
                    }
                }
            }

            if restored > 0 {
                info!("Successfully reattached to {} container", restored);
            }
        }
    }
}
