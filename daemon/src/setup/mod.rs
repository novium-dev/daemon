// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::{ArchiveCompression, CompressionLevel};
use crate::config::{
    BackupEntry, DaemonConfig, NetworkConfig, SSHConfig, SSHHostKey, StorageEntry,
};
use async_trait::async_trait;
use axum::http::HeaderValue;
use axum_server::Server;
use axum_server::tls_rustls::{RustlsAcceptor, RustlsConfig};
use core::error::Error;
use core::net::{IpAddr, Ipv6Addr, SocketAddr};
use core::time::Duration;
use daemon::config::MountPoint;
use daemon::internal::middleware;
use daemon::key::{DaemonCertResolver, TlsAcceptor};
use daemon_common::key::rcgen::{PKCS_ECDSA_P256_SHA256, PKCS_RSA_SHA256};
use daemon_common::key::{DaemonKey, DaemonRootKey};
use daemon_common::node::{
    BackendNodeConfig, BackupStorage, NodeRegistrationRequest, ServerStorage, SetupNodeConfig,
};
use daemon_common::service::framework_router;
use daemon_common::service::setup::SetupService;
use daemon_common::service::setup::setup_service_error::ConfigError;
use reqwest::blocking::Client;
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, CertificateSigningRequestDer};
use rustls::server::WebPkiClientVerifier;
use sqlx::{Connection as _, MySqlConnection, QueryBuilder, query_scalar};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use tokio::runtime::Handle;
use tracing::{info, warn};
use url::Url;
use utoipa_axum::router::OpenApiRouter;
use uuid::Uuid;

pub(crate) fn setup(
    skip_mariadb_plugin: bool,
    url: &str,
    code: &str,
    database_url: &Url,
    port: u16,
) {
    info!("Starting Setup...");

    let backend = Url::parse(url).expect("SETUP_URL is not a valid URL");

    info!("Retrieving daemon root key...");
    let Some(root) = read_root_key(&backend, code).expect("failed to retrieve daemon root key")
    else {
        return;
    };
    let root = Arc::new(root);

    info!("Retrieving node config...");
    let node = read_config(&backend, code).expect("failed to retrieve node config");

    let config = node_to_daemon_config(&backend, &node, database_url, port);

    let keypair = crate::common::key::rcgen::KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
        .expect("failed to generate key pair");

    let csr =
        DaemonKey::new_request(&keypair).expect("failed to build certificate signing request");
    let temp_key = Arc::new(DaemonKey::new_temp(&keypair).expect("failed to build temporary key"));

    test_host(&config, &root, &temp_key, Uuid::nil());

    if !skip_mariadb_plugin {
        ensure_mariadb_plugin(database_url.as_str());
    }

    let request = config_to_register_request(
        temp_key
            .cert
            .first()
            .expect("temporary key has at least one certificate"),
        &csr,
    );
    let response = Client::default()
        .post(
            backend
                .join("/api/internal/daemon/register")
                .expect("register endpoint url is valid"),
        )
        .bearer_auth(code)
        .json(&request)
        .send()
        .expect("failed to send registration request")
        .error_for_status()
        .expect("registration request returned an error status");
    let bytes = response
        .bytes()
        .expect("failed to read registration response body")
        .to_vec();
    let signed = CertificateDer::from(bytes);
    let key = DaemonKey::new(&root, signed, temp_key.privkey.clone_key())
        .expect("failed to build signed daemon key");

    // Persist the configuration and keys only after registration succeeds, so an interrupted or
    // failed run can simply be retried without having to manually delete config.toml first.
    std::fs::write(
        "config.toml",
        toml::to_string(&config).expect("failed to serialize daemon config"),
    )
    .expect("failed to write config.toml");
    std::fs::write("daemon_root.pem", root.dump(None, None))
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

/// Installs the `MariaDB` authentication plugin into the configured database if it is missing.
fn ensure_mariadb_plugin(mysql: &str) {
    Handle::current()
        .block_on(async {
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
        })
        .expect("failed to ensure MariaDB authentication plugin is installed");
}

fn config_to_register_request(
    cert: &CertificateDer<'static>,
    csr: &CertificateSigningRequestDer<'static>,
) -> NodeRegistrationRequest {
    NodeRegistrationRequest {
        cert: cert.to_vec(),
        csr: csr.to_vec(),
        default_bind: "0.0.0.0".to_owned(),
    }
}

fn test_host(config: &DaemonConfig, root: &Arc<DaemonRootKey>, key: &Arc<DaemonKey>, uuid: Uuid) {
    #[derive(Clone)]
    struct ConnectionTestWrapper;

    #[async_trait]
    impl SetupService for ConnectionTestWrapper {
        async fn config(&self) -> Result<BackendNodeConfig, ConfigError> {
            Err(ConfigError::AwaitingSetup)
        }
    }

    let mut rustls = ServerConfig::builder()
        .with_client_cert_verifier(
            WebPkiClientVerifier::builder(Arc::clone(&root.cert_store))
                .allow_unauthenticated()
                .allow_unknown_revocation_status()
                .build()
                .expect("failed to build client certificate verifier"),
        )
        .with_cert_resolver(Arc::new(DaemonCertResolver(Arc::clone(key), None, uuid)));
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

fn read_root_key(url: &Url, code: &str) -> Result<Option<DaemonRootKey>, anyhow::Error> {
    let response = Client::default()
        .get(
            url.join("/api/internal/daemon/root_key")
                .expect("root_key endpoint url is valid"),
        )
        .bearer_auth(code)
        .send()?;
    if response.status() == axum::http::StatusCode::UNAUTHORIZED {
        warn!("Code is invalid");
        return Ok(None);
    }
    let str: String = response.json().map_err(std::io::Error::other)?;
    Ok(Some(DaemonRootKey::parse(str)?.0))
}

fn read_config(url: &Url, code: &str) -> Result<SetupNodeConfig, anyhow::Error> {
    let response = Client::default()
        .get(
            url.join("/api/internal/daemon/config")
                .expect("config endpoint url is valid"),
        )
        .bearer_auth(code)
        .send()?
        .error_for_status()?;
    Ok(response.json()?)
}
