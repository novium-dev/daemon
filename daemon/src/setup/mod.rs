// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::{ArchiveCompression, CompressionLevel};
use crate::config::{
    BackupEntry, DaemonConfig, NetworkConfig, SSHConfig, SSHHostKey, StorageEntry,
};
use async_trait::async_trait;
use axum::http::HeaderValue;
use core::net::{IpAddr, Ipv6Addr, SocketAddr};
use daemon::config::{LimitConfig, MountPoint};
use daemon::internal::middleware;
use daemon_common::DAEMON_KEY_PREFIX;
use daemon_common::node::{
    BackendNodeConfig, BackupStorage, NodeRegistrationRequest, ServerStorage, SetupNodeConfig,
};
use daemon_common::service::framework_router;
use daemon_common::service::setup::SetupService;
use daemon_common::service::setup::setup_service_error::ConfigError;
use reqwest::Client;
use std::collections::HashSet;
use std::path::PathBuf;
use tracing::{info, warn};
use url::Url;
use utoipa_axum::router::OpenApiRouter;
use zelus_keys::setup::{PendingKey, SetupClient, serve_connection_test};

pub(crate) async fn setup(
    skip_mariadb_plugin: bool,
    url: &str,
    code: &str,
    database_url: &Url,
    port: u16,
) {
    info!("Starting Setup...");

    let backend = Url::parse(url).expect("SETUP_URL is not a valid URL");
    let client = SetupClient::new(Client::default(), &backend, "/api/internal/daemon", code)
        .expect("setup endpoint url is valid");

    info!("Retrieving daemon root key...");
    let Some(offer) = client
        .root_key()
        .await
        .expect("failed to retrieve daemon root key")
    else {
        warn!("Code is invalid");
        return;
    };

    info!("Retrieving node config...");
    let node: SetupNodeConfig = client
        .config()
        .await
        .expect("failed to retrieve node config");

    let config = node_to_daemon_config(&backend, &node, database_url, port);

    let pending = PendingKey::generate(offer.prefers_rsa, DAEMON_KEY_PREFIX)
        .expect("failed to generate the node key");

    serve_connection_test(
        config.listeners.clone(),
        &offer.root,
        &pending.temp,
        connection_test_router(),
    )
    .await
    .expect("failed to start the temporary setup server");

    #[cfg(feature = "databases")]
    if !skip_mariadb_plugin {
        ensure_mariadb_plugin(database_url.as_str()).await;
    }
    #[cfg(not(feature = "databases"))]
    let _ = skip_mariadb_plugin;

    let signed = client
        .register(&NodeRegistrationRequest {
            cert: pending.certificate().to_vec(),
            csr: pending.csr.to_vec(),
            default_bind: "0.0.0.0".to_owned(),
        })
        .await
        .expect("failed to register the node");
    let key = pending
        .finish(&offer.root, signed)
        .expect("failed to build signed daemon key");

    std::fs::write(
        "config.toml",
        toml::to_string(&config).expect("failed to serialize daemon config"),
    )
    .expect("failed to write config.toml");
    std::fs::write("daemon_root.pem", offer.root.dump(None, None))
        .expect("failed to write daemon_root.pem");
    std::fs::write("daemon.pem", key.dump()).expect("failed to write daemon.pem");

    info!("Done");
}

fn node_to_daemon_config(
    backend: &Url,
    node: &SetupNodeConfig,
    database_url: &Url,
    port: u16,
) -> DaemonConfig {
    let mut mounts = Vec::new();
    let backend_root = backend.join("/").expect("backend root url is valid");

    if let Some((_, socket)) = database_url
        .query_pairs()
        .find(|(param, _)| param.eq("socket"))
    {
        let src = PathBuf::from(socket.to_string());
        let src = src.parent().expect("socket path has a parent directory");
        mounts.push(MountPoint {
            src: src.to_string_lossy().to_string(),
            dest: "/var/run/mysqld".to_owned(),
            mode: "ro".to_owned(),
        });
    }

    DaemonConfig {
        database: database_url.to_string(),
        public_url: node.base_url.clone(),
        listeners: HashSet::from([SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port)]),
        origins: node
            .origins
            .iter()
            .map(|origin| {
                HeaderValue::from_str(origin).expect("backend provided a valid origin header value")
            })
            .collect(),
        backend_url: backend_root.clone(),
        shutdown_stop_servers: false,
        tls: None,
        ssh: node.ssh.as_ref().map(|public| SSHConfig {
            listeners: HashSet::from([SocketAddr::new(
                IpAddr::V6(Ipv6Addr::UNSPECIFIED),
                public.port,
            )]),
            host_key: vec![
                SSHHostKey::Ed25519 {
                    path: "ed25519.pem".to_owned(),
                },
                SSHHostKey::Rsa {
                    path: "rsa.pem".to_owned(),
                },
            ],
            auth_url: backend_root.clone(),
            auth_cache_duration: 30 * 1000,
        }),
        storage: node
            .storages
            .iter()
            .map(|(name, storage)| (name.clone(), server_storage_to_entry(name, storage)))
            .collect(),
        backup: node
            .backups
            .iter()
            .map(|(name, backup)| (name.clone(), backup_storage_to_entry(name, backup)))
            .collect(),
        network: NetworkConfig::default(),
        limit: LimitConfig::default(),
        mounts,
    }
}

fn server_storage_to_entry(name: &str, storage: &ServerStorage) -> StorageEntry {
    match storage {
        ServerStorage::Directory => StorageEntry::Directory {
            path: format!("./storages/{name}"),
        },
        #[cfg(feature = "btrfs")]
        ServerStorage::Btrfs(group) => StorageEntry::Btrfs {
            group: group.clone(),
            path: format!("./storages/{name}"),
            quota: true,
        },
    }
}

fn backup_storage_to_entry(name: &str, backup: &BackupStorage) -> BackupEntry {
    match backup {
        BackupStorage::Archive => BackupEntry::Archive {
            compression: ArchiveCompression::Zstd {
                level: CompressionLevel::Best,
            },
            path: format!("./backups/{name}"),
            order: Some(0),
        },
        #[cfg(feature = "btrfs")]
        BackupStorage::Btrfs(group) => BackupEntry::Btrfs {
            jit_compression: ArchiveCompression::Zstd {
                level: CompressionLevel::Best,
            },
            path: format!("./backups/{name}"),
            group: group.clone(),
            quota: true,
            order: Some(0),
        },
    }
}

#[cfg(feature = "databases")]
async fn ensure_mariadb_plugin(mysql: &str) {
    use core::error::Error;
    use sqlx::{Connection as _, MySqlConnection, QueryBuilder, query_scalar};

    async {
        let mut conn = MySqlConnection::connect(mysql).await?;

        let plugins: HashSet<String> =
            query_scalar!("SELECT `PLUGIN_NAME` FROM `information_schema`.`plugins`")
                .fetch_all(&mut conn)
                .await?
                .into_iter()
                .collect();

        if !plugins.contains(crate::common::MARIADB_PLUGIN_NAME) {
            info!("MariaDB Authentication plugin missing! Trying to install it...");
            let mut query = QueryBuilder::new("INSTALL PLUGIN ");
            query.push(crate::common::MARIADB_PLUGIN_NAME);
            query.push("  SONAME 'libdaemon_mariadb_plugin.so'");
            query.build().execute(&mut conn).await?;
        }

        Ok::<(), Box<dyn Error + Send + Sync>>(())
    }
    .await
    .expect("failed to ensure MariaDB authentication plugin is installed");
}

fn connection_test_router() -> axum::Router {
    #[derive(Clone)]
    struct ConnectionTestWrapper;

    #[async_trait]
    impl SetupService for ConnectionTestWrapper {
        async fn config(&self) -> Result<BackendNodeConfig, ConfigError> {
            Err(ConfigError::AwaitingSetup)
        }
    }

    let (router,) =
        framework_router!(ConnectionTestWrapper ConnectionTestWrapper (default,) { SetupService });
    OpenApiRouter::new()
        .merge(router.into_openapi())
        .layer(axum::middleware::from_fn(middleware))
        .split_for_parts()
        .0
}
