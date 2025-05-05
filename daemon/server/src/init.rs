use crate::config::DaemonConfig;
use crate::server::FetchedServer;
use crate::server::watcher::attach;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageError, StorageLockData, storage_read};
use crate::sync::Mutex;
use crate::{Daemon, ctlsock, schedule, sql, storage};
use bollard::models::{Ipam, IpamConfig};
use bollard::network::CreateNetworkOptions;
use ed25519_dalek::SigningKey;
use libc::{SIGINT, SIGTERM};
use reqwest::Client;
use signal_hook::iterator::Signals;
use sqlx::mysql::MySqlPoolOptions;
use sqlx::query;
use std::collections::HashMap;
use std::io::ErrorKind;
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
    pub async fn init(config: DaemonConfig) -> Result<Self, DaemonInitError> {
        Self::init_embedded(
            config,
            Url::parse("http://localhost:3000").unwrap(),
            SigningKey::from_bytes(&[0; 32]),
        )
        .await
    }

    pub async fn init_embedded(
        config: DaemonConfig,
        base_url: Url,
        user_key: SigningKey,
    ) -> Result<Self, DaemonInitError> {
        let servers = HashMap::new();

        info!("Connecting to Docker...");
        let docker = bollard::Docker::connect_with_local_defaults()
            .inspect_err(|_err| error!("Failed to connect to docker"))
            .unwrap();

        match docker
            .inspect_network::<String>(crate::common::NETWORK_DEFAULT, None)
            .await
        {
            Ok(_) => {}
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {
                docker
                    .create_network(CreateNetworkOptions {
                        name: crate::common::NETWORK_DEFAULT.to_string(),
                        options: HashMap::from([
                            (
                                "com.docker.network.bridge.enable_icc".to_string(),
                                false.to_string(),
                            ),
                            (
                                "com.docker.network.bridge.enable_ip_masquerade".to_string(),
                                false.to_string(),
                            ),
                        ]),
                        internal: false,
                        enable_ipv6: true,
                        ipam: Ipam {
                            config: Some(vec![IpamConfig {
                                subnet: Some(crate::common::BRIDGE_DEFAULT_IPV6_PREFIX.to_string()),
                                ..IpamConfig::default()
                            }]),
                            ..Ipam::default()
                        },
                        ..CreateNetworkOptions::default()
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
            .inspect_network::<String>(crate::common::NETWORK_INTERNET, None)
            .await
        {
            Ok(_) => {}
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => {
                docker
                    .create_network(CreateNetworkOptions {
                        name: crate::common::NETWORK_INTERNET.to_string(),
                        options: HashMap::from([
                            (
                                "com.docker.network.bridge.enable_icc".to_string(),
                                false.to_string(),
                            ),
                            (
                                "com.docker.network.bridge.enable_ip_masquerade".to_string(),
                                true.to_string(),
                            ),
                        ]),
                        internal: false,
                        enable_ipv6: true,
                        ipam: Ipam {
                            config: Some(vec![IpamConfig {
                                subnet: Some(
                                    crate::common::BRIDGE_INTERNET_IPV6_PREFIX.to_string(),
                                ),
                                ..IpamConfig::default()
                            }]),
                            ..Ipam::default()
                        },
                        ..CreateNetworkOptions::default()
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
            shutdown_task: Arc::new(Mutex::new(JoinSet::new())),
            schedule_task: Arc::new(Mutex::new(JoinSet::new())),
            schedules: Arc::new(Mutex::new(HashMap::new())),
            docker: Arc::new(docker),
            conn: Arc::new(conn),
            client: Arc::new(Client::default()),
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
            .server_task
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
        debug!("Shutting daemon down...");
        let _ = self.shutdown_channel.send(());
        self.server_task.lock().await.shutdown().await;
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
        set.join_all().await;
        debug!("Daemon shutdown complete");
        tokio::time::sleep(Duration::from_millis(250)).await;
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
    }

    async fn reattach_container(&self) {
        if let Ok(containers) = self
            .docker
            .list_containers::<String>(None)
            .await
            .inspect_err(|err| {
                warn!(
                    "Failed to scan for running containers... (assuming there are none): {:?}",
                    err
                )
            })
        {
            let mut servers = self.servers.lock().await;

            let mut restored = 0;
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
                    .fetch_optional(self.conn.deref())
                    .await
                    .unwrap()
                    {
                        Some(row) => {
                            let mut server_data = server_data_arc.lock().await;
                            let StorageLockData {
                                lock: storage_lock,
                                value: storage_value,
                                stats_lock: storage_stats_lock,
                                ..
                            } = storage_read::<StorageError>(
                                &self.config,
                                self.conn.deref(),
                                &self.storages,
                                StorageIdentifier::Primary(id),
                            )
                            .await
                            .unwrap();

                            let cpu_max = row.cpu.map(|cpu| cpu * 100f32).unwrap_or(-1f32);
                            let ram_max = row.ram.map(|ram| ram as i64).unwrap_or(-1);
                            let swap_max = row.swap.map(|swap| swap as i64).unwrap_or(-1);

                            if let Err(err) = attach(
                                id,
                                self,
                                cpu_max,
                                ram_max,
                                swap_max,
                                server_data.deref_mut(),
                                server_data_arc.clone(),
                                storage_lock,
                                storage_value,
                                storage_stats_lock,
                            )
                            .await
                            {
                                warn!("Failed to reattach server {}: {:?}", id, err);
                            } else {
                                restored += 1;
                            }
                        }
                        None => {
                            warn!(
                                "Found server that is missing in database: {}. Killing silently...",
                                id
                            );
                            let _ = self
                                .docker
                                .kill_container::<String>(&FetchedServer::container_name(id), None)
                                .await;
                        }
                    }
                }
            }

            if restored > 0 {
                info!("Successfully reattached to {} container", restored);
            }
        }
    }
}
