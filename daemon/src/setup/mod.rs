// SPDX-License-Identifier: AGPL-3.0-only
#![allow(clippy::unwrap_used, reason = "setup is a fail-fast provisioning step")]
#![allow(clippy::print_stdout, reason = "setup talks to the operator via stdout")]
#![allow(clippy::uninlined_format_args, reason = "setup wizard")]
#![allow(clippy::indexing_slicing, reason = "setup wizard")]
#![allow(clippy::clone_on_ref_ptr, reason = "setup wizard")]
#![allow(clippy::std_instead_of_core, reason = "setup wizard")]
#![allow(clippy::unused_trait_names, reason = "setup wizard")]

use crate::config::{ArchiveCompression, CompressionLevel};
use crate::config::{
    BackupEntry, DaemonConfig, MountPoint, NetworkConfig, SSHConfig, SSHHostKey, StorageEntry,
};
use async_trait::async_trait;
use axum::http::HeaderValue;
use axum_server::Server;
use axum_server::tls_rustls::{RustlsAcceptor, RustlsConfig};
use daemon::internal::middleware;
use daemon::key::{DaemonCertResolver, TlsAcceptor};
use daemon_common::key::rcgen::{PKCS_ECDSA_P256_SHA256, PKCS_RSA_SHA256};
use daemon_common::key::{DaemonKey, DaemonRootKey};
use daemon_common::node::{BackupStorage, NodeConfig, NodeRegistrationRequest, ServerStorage};
use daemon_common::service::framework_router;
use daemon_common::service::setup::SetupService;
use daemon_common::service::setup::setup_service_error::{ConfigError, ConnectionTestError};
use reqwest::blocking::Client;
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, CertificateSigningRequestDer};
use rustls::server::WebPkiClientVerifier;
use sqlx::{Connection, MySqlConnection, QueryBuilder, query_scalar};
use std::collections::HashSet;
use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::runtime::Handle;
use tracing::info;
use url::Url;
use utoipa_axum::router::OpenApiRouter;
use uuid::Uuid;

/// Runs the non-interactive setup, driven by the `SETUP_URL` and `SETUP_CODE` environment variables.
pub(crate) fn setup(skip_mariadb_plugin: bool) {
    info!("Starting Setup...");

    let url = std::env::var("SETUP_URL").expect("SETUP_URL environment variable is required");
    let code = std::env::var("SETUP_CODE").expect("SETUP_CODE environment variable is required");
    let backend = Url::parse(&url).expect("SETUP_URL is not a valid URL");

    println!("Retrieving daemon root key...");
    let Some((root, rsa)) = read_root_key(&backend, &code).unwrap() else {
        return;
    };
    let root = Arc::new(root);

    println!("Retrieving node config...");
    let node = read_config(&backend, &code).unwrap();

    let config = node_to_daemon_config(&backend, &node);
    {
        let mut file = File::create("config.toml").unwrap();
        file.write_all(toml::to_string(&config).unwrap().as_bytes())
            .unwrap();
        file.flush().unwrap();
    }

    let keypair = crate::common::key::rcgen::KeyPair::generate_for(if rsa {
        &PKCS_RSA_SHA256
    } else {
        &PKCS_ECDSA_P256_SHA256
    })
    .unwrap();

    let csr = DaemonKey::new_request(&keypair).unwrap();
    let temp_key = Arc::new(DaemonKey::new_temp(&keypair).unwrap());

    test_host(&config, root.clone(), temp_key.clone(), Uuid::nil());

    let request = config_to_register_request(&node, &temp_key.cert[0], &csr, skip_mariadb_plugin);
    let response = Client::default()
        .post(backend.join("/internal/daemon/register").unwrap())
        .bearer_auth(&code)
        .json(&request)
        .send()
        .unwrap()
        .error_for_status()
        .unwrap();
    let bytes = response.bytes().unwrap().to_vec();
    let signed = CertificateDer::from(bytes);
    let key = DaemonKey::new(&root, signed, temp_key.privkey.clone_key()).unwrap();

    {
        let mut file = File::create("daemon_root.pem").unwrap();
        file.write_all(root.dump(None, None).as_bytes()).unwrap();
        file.flush().unwrap();
    }
    {
        let mut file = File::create("daemon.pem").unwrap();
        file.write_all(key.dump().as_bytes()).unwrap();
        file.flush().unwrap();
    }

    println!("Done");
}

/// Builds the local [`DaemonConfig`] from the backend-provided logical [`NodeConfig`], filling in
/// local specifics (bind ports, on-disk paths, host-key paths, mounts) from defaults.
fn node_to_daemon_config(backend: &Url, node: &NodeConfig) -> DaemonConfig {
    let mut mounts = Vec::new();
    let mysql_url = Url::parse(&node.database).expect("database url provided by backend is invalid");
    if let Some((_, socket)) = mysql_url.query_pairs().find(|(param, _)| param.eq("socket")) {
        let src = PathBuf::from(socket.to_string());
        let src = src.parent().unwrap();
        mounts.push(MountPoint {
            src: src.to_string_lossy().to_string(),
            dest: "/var/run/mysqld".to_string(),
            mode: "ro".to_string(),
        });
    }

    let backend_root = backend.join("/").unwrap();

    DaemonConfig {
        database: node.database.clone(),
        public_url: node.base_url.clone(),
        listeners: HashSet::from([SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 3000)]),
        origin: HeaderValue::from_str(backend_root.as_str()).unwrap(),
        backend_url: backend_root.clone(),
        tls: None,
        ssh: node.ssh.as_ref().map(|public| SSHConfig {
            listeners: HashSet::from([SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 22)]),
            host_key: vec![
                SSHHostKey::Ed25519 {
                    path: "ed25519.pem".to_string(),
                },
                SSHHostKey::Rsa {
                    path: "rsa.pem".to_string(),
                },
            ],
            auth_url: backend_root.clone(),
            auth_cache_duration: 30 * 1000,
            public: public.clone(),
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

fn config_to_register_request(
    node: &NodeConfig,
    cert: &CertificateDer<'static>,
    csr: &CertificateSigningRequestDer<'static>,
    no_plugin: bool,
) -> NodeRegistrationRequest {
    if !no_plugin {
        let mysql = node.database.clone();
        Handle::current()
            .block_on(async {
                let mut conn = MySqlConnection::connect(&mysql).await?;

                let plugins: HashSet<String> =
                    query_scalar!("SELECT `PLUGIN_NAME` FROM `information_schema`.`plugins`")
                        .fetch_all(&mut conn)
                        .await?
                        .into_iter()
                        .collect();

                if !plugins.contains(crate::common::MARIADB_PLUGIN_NAME) {
                    println!("MariaDB Authentication plugin missing! Trying to install it...");
                    let mut query = QueryBuilder::new("INSTALL PLUGIN ");
                    query.push(crate::common::MARIADB_PLUGIN_NAME);
                    query.push("  SONAME 'libdaemon_mariadb_plugin.so'");
                    query.build().execute(&mut conn).await?;
                }

                Ok::<(), Box<dyn Error + Send + Sync>>(())
            })
            .unwrap();
    }

    NodeRegistrationRequest {
        base_url: node.base_url.clone(),
        ssh: node.ssh.clone(),
        cert: cert.to_vec(),
        csr: csr.to_vec(),
        storages: node.storages.clone(),
        backups: node.backups.clone(),
        default_bind: "0.0.0.0".to_string(),
        default_host: node.base_url.host_str().unwrap().to_string(),
    }
}

fn test_host(config: &DaemonConfig, root: Arc<DaemonRootKey>, key: Arc<DaemonKey>, uuid: Uuid) {
    #[derive(Clone)]
    struct ConnectionTestWrapper;

    #[async_trait]
    impl SetupService for ConnectionTestWrapper {
        async fn connection_test(&self) -> Result<(), ConnectionTestError> {
            Err(ConnectionTestError::AwaitingSetup)
        }

        async fn config(&self) -> Result<NodeConfig, ConfigError> {
            Err(ConfigError::AwaitingSetup)
        }
    }

    let mut rustls = ServerConfig::builder()
        .with_client_cert_verifier(
            WebPkiClientVerifier::builder(root.cert_store.clone())
                .allow_unauthenticated()
                .allow_unknown_revocation_status()
                .build()
                .unwrap(),
        )
        .with_cert_resolver(Arc::new(DaemonCertResolver(key.clone(), None, uuid)));
    rustls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let rustls = RustlsConfig::from_config(Arc::new(rustls));

    for listener in config.listeners.clone() {
        let (router,) = framework_router!(ConnectionTestWrapper ConnectionTestWrapper (default,) { SetupService });
        let router = OpenApiRouter::new()
            .merge(router.into_openapi())
            .layer(axum::middleware::from_fn(middleware));
        let (router, _) = router.split_for_parts();

        tokio::task::spawn(
            Server::bind(listener)
                .acceptor(TlsAcceptor(RustlsAcceptor::new(rustls.clone())))
                .serve(router.into_make_service()),
        );
    }

    thread::sleep(Duration::from_secs(5));
}

fn read_root_key(url: &Url, code: &str) -> Result<Option<(DaemonRootKey, bool)>, anyhow::Error> {
    let response = Client::default()
        .get(url.join("/internal/daemon/root_key").unwrap())
        .bearer_auth(code)
        .send()?;
    if response.status() == axum::http::StatusCode::UNAUTHORIZED {
        println!("Code is invalid");
        return Ok(None);
    }
    let rsa = response.headers().contains_key("X-Prefer-RSA-Key");
    let str: String = response.json().map_err(std::io::Error::other)?;
    Ok(Some((DaemonRootKey::parse(str)?.0, rsa)))
}

fn read_config(url: &Url, code: &str) -> Result<NodeConfig, anyhow::Error> {
    let response = Client::default()
        .get(url.join("/internal/daemon/config").unwrap())
        .bearer_auth(code)
        .send()?
        .error_for_status()?;
    Ok(response.json()?)
}
