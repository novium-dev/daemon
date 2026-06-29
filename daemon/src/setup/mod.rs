// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::{ArchiveCompression, CompressionLevel};
use crate::config::{
    BackupEntry, DaemonConfig, NetworkConfig, SSHConfig, SSHHostKey, StorageEntry,
};
use async_trait::async_trait;
use axum::http::HeaderValue;
use axum_server::Server;
use axum_server::tls_rustls::{RustlsAcceptor, RustlsConfig};
use daemon::internal::middleware;
use daemon::key::{DaemonCertResolver, TlsAcceptor};
use daemon_common::key::rcgen::{PKCS_ECDSA_P256_SHA256, PKCS_RSA_SHA256};
use daemon_common::key::{DaemonKey, DaemonRootKey};
use daemon_common::node::{BackupStorage, BackendNodeConfig, SetupNodeConfig, NodeRegistrationRequest, ServerStorage};
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
use daemon::config::MountPoint;

pub(crate) fn setup(skip_mariadb_plugin: bool, url: &str, code: &str, database_url: Url) {
    info!("Starting Setup...");

    let backend = Url::parse(url).expect("SETUP_URL is not a valid URL");

    println!("Retrieving daemon root key...");
    let Some((root, rsa)) = read_root_key(&backend, code).unwrap() else {
        return;
    };
    let root = Arc::new(root);

    println!("Retrieving node config...");
    let node = read_config(&backend, code).unwrap();

    let config = node_to_daemon_config(&backend, &node, &database_url);
    write_file("config.toml", toml::to_string(&config).unwrap().as_bytes());

    let keypair = crate::common::key::rcgen::KeyPair::generate_for(if rsa {
        &PKCS_RSA_SHA256
    } else {
        &PKCS_ECDSA_P256_SHA256
    })
    .unwrap();

    let csr = DaemonKey::new_request(&keypair).unwrap();
    let temp_key = Arc::new(DaemonKey::new_temp(&keypair).unwrap());

    test_host(&config, &root, &temp_key, Uuid::nil());

    let request = config_to_register_request(&node, &temp_key.cert[0], &csr, database_url.to_string(), skip_mariadb_plugin);
    let response = Client::default()
        .post(backend.join("/internal/daemon/register").unwrap())
        .bearer_auth(code)
        .json(&request)
        .send()
        .unwrap()
        .error_for_status()
        .unwrap();
    let bytes = response.bytes().unwrap().to_vec();
    let signed = CertificateDer::from(bytes);
    let key = DaemonKey::new(&root, signed, temp_key.privkey.clone_key()).unwrap();

    write_file("daemon_root.pem", root.dump(None, None).as_bytes());
    write_file("daemon.pem", key.dump().as_bytes());

    println!("Done");
}

/// Writes `contents` to `path`, flushing before the handle is dropped.
fn write_file(path: &str, contents: &[u8]) {
    let mut file = File::create(path).unwrap();
    file.write_all(contents).unwrap();
    file.flush().unwrap();
}

fn node_to_daemon_config(backend: &Url, node: &SetupNodeConfig, database_url: &Url) -> DaemonConfig {
    let mut mounts = Vec::new();
    let backend_root = backend.join("/").unwrap();

    if let Some((_, socket)) = database_url
        .query_pairs()
        .find(|(param, _)| param.eq("socket"))
    {
        let src = PathBuf::from(socket.to_string());
        let src = src.parent().unwrap();
        mounts.push(MountPoint {
            src: src.to_string_lossy().to_string(),
            dest: "/var/run/mysqld".to_owned(),
            mode: "ro".to_owned(),
        });
    }

    DaemonConfig {
        database: database_url.to_string(),
        public_url: node.base_url.clone(),
        listeners: HashSet::from([SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 3000)]),
        origin: HeaderValue::from_str(backend_root.as_str()).unwrap(),
        backend_url: backend_root.clone(),
        tls: None,
        ssh: node.ssh.as_ref().map(|public| SSHConfig {
            listeners: HashSet::from([SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 22)]),
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
    node: &SetupNodeConfig,
    cert: &CertificateDer<'static>,
    csr: &CertificateSigningRequestDer<'static>,
    mysql: String,
    no_plugin: bool,
) -> NodeRegistrationRequest {
    if !no_plugin {
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
        cert: cert.to_vec(),
        csr: csr.to_vec(),
        default_bind: "0.0.0.0".to_owned(),
        default_host: node.base_url.host_str().unwrap().to_owned(),
        config: node.clone(),
    }
}

fn test_host(config: &DaemonConfig, root: &Arc<DaemonRootKey>, key: &Arc<DaemonKey>, uuid: Uuid) {
    #[derive(Clone)]
    struct ConnectionTestWrapper;

    #[async_trait]
    impl SetupService for ConnectionTestWrapper {
        async fn connection_test(&self) -> Result<(), ConnectionTestError> {
            Err(ConnectionTestError::AwaitingSetup)
        }

        async fn config(&self) -> Result<BackendNodeConfig, ConfigError> {
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

        info!("Starting temporary setup server on {listener}...");

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

fn read_config(url: &Url, code: &str) -> Result<SetupNodeConfig, anyhow::Error> {
    let response = Client::default()
        .get(url.join("/internal/daemon/config").unwrap())
        .bearer_auth(code)
        .send()?
        .error_for_status()?;
    Ok(response.json()?)
}
