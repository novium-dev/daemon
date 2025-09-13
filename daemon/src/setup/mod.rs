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
#![expect(clippy::unwrap_used, reason = "this is a garbage implementation")]
#![expect(clippy::print_stdout, reason = "this is a garbage implementation")]
#![expect(
    clippy::single_char_pattern,
    reason = "this is a garbage implementation"
)]
#![expect(
    clippy::uninlined_format_args,
    reason = "this is a garbage implementation"
)]
#![expect(clippy::map_unwrap_or, reason = "this is a garbage implementation")]
#![expect(clippy::str_to_string, reason = "this is a garbage implementation")]
#![expect(clippy::redundant_clone, reason = "this is a garbage implementation")]
#![expect(
    clippy::needless_pass_by_value,
    reason = "this is a garbage implementation"
)]
#![expect(
    clippy::verbose_file_reads,
    reason = "this is a garbage implementation"
)]
#![expect(
    clippy::semicolon_outside_block,
    reason = "this is a garbage implementation"
)]
#![expect(clippy::indexing_slicing, reason = "this is a garbage implementation")]
#![expect(clippy::clone_on_ref_ptr, reason = "this is a garbage implementation")]
#![expect(
    clippy::unnecessary_to_owned,
    reason = "this is a garbage implementation"
)]
#![expect(
    clippy::redundant_pub_crate,
    reason = "this is a garbage implementation"
)]
#![expect(
    clippy::semicolon_if_nothing_returned,
    reason = "this is a garbage implementation"
)]
#![expect(
    clippy::std_instead_of_core,
    reason = "this is a garbage implementation"
)]
#![expect(
    clippy::unused_trait_names,
    reason = "this is a garbage implementation"
)]

use crate::config::{ArchiveCompression, CompressionLevel};
use crate::config::{BackupEntry, DaemonConfig, MountPoint, NetworkConfig, StorageEntry};
use async_trait::async_trait;
use axum::http::HeaderValue;
use axum_server::Server;
use axum_server::tls_rustls::{RustlsAcceptor, RustlsConfig};
use daemon::internal::middleware;
use daemon::key::{DaemonCertResolver, TlsAcceptor};
use daemon_common::key::rcgen::{PKCS_ECDSA_P256_SHA256, PKCS_RSA_SHA256};
use daemon_common::key::{DaemonKey, DaemonRootKey};
use daemon_common::node::{
    BackupStorage, HostAndPort, NodeRegistrationRequest, NodeUpdateRequest, ServerStorage,
};
use daemon_common::service::framework_router;
use daemon_common::service::setup::SetupService;
use daemon_common::service::setup::error::ConnectionTestError;
use inquire::autocompletion::Replacement;
use inquire::validator::{ErrorMessage, StringValidator, Validation};
use inquire::{Autocomplete, CustomUserError, Text};
use lazy_regex::{Regex, regex};
use reqwest::blocking::Client;
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, CertificateSigningRequestDer};
use rustls::server::WebPkiClientVerifier;
use sqlx::{Connection, MySqlConnection, QueryBuilder, query_scalar};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;
use std::process::Stdio;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use std::{io, thread};
use tokio::runtime::Handle;
use tracing::info;
use url::Host;
use url::Url;
use utoipa_axum::router::OpenApiRouter;
use uuid::Uuid;

#[derive(Copy, Clone)]
struct MySQLValidator;

#[derive(Copy, Clone)]
struct URLValidator<const SECURE: bool>;

#[derive(Copy, Clone)]
struct HostPortValidator;

#[derive(Clone)]
struct RegexValidator(&'static Regex);

pub(crate) fn setup(backend_url: Option<Url>, code: Option<String>, skip_mariadb_plugin: bool) {
    info!("Starting Setup Wizard...");

    match File::open("config.toml") {
        Ok(file) => {
            println!(
                "\nThere already is an existing configuration for the daemon? Do you want to apply config updates or start a new setup?"
            );
            if Text::new(" ")
                .with_validator(RegexValidator(regex!("Update|New")))
                .with_autocomplete(VecAutoComplete(&["Update", "New"]))
                .prompt()
                .unwrap()
                .eq("New")
            {
                new(backend_url, code, skip_mariadb_plugin)
            } else {
                update(file, backend_url, code)
            }
        }
        Err(_) => new(backend_url, code, skip_mariadb_plugin),
    }
}

fn default(backend_url: Option<Url>, code: Option<String>) -> (Url, String) {
    let backend = backend_url.unwrap_or_else(|| {
        println!(
            "\nPlease enter the url to the backend.\nThe API should be reachable using the url."
        );
        Url::parse(
            &Text::new("URL: ")
                .with_validator(URLValidator::<false>)
                .prompt()
                .unwrap(),
        )
        .unwrap()
    });
    let code = code.unwrap_or_else(|| {
        println!("\nPlease enter your registration code");
        Text::new("Code: ").prompt().unwrap()
    });

    (backend, code)
}

pub(crate) fn update(mut file: File, backend_url: Option<Url>, code: Option<String>) {
    let (backend, code) = default(backend_url, code);
    let config: DaemonConfig = {
        let mut str = String::new();
        file.read_to_string(&mut str).unwrap();
        drop(file);
        toml::from_str(&str).unwrap()
    };

    let root = {
        let mut file = File::open("daemon_root.pem").unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        DaemonRootKey::parse(&mut buf).unwrap().0
    };
    let root = Arc::new(root);
    let key = {
        let mut file = File::open("daemon.pem").unwrap();
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        DaemonKey::parse(&root, &buf).unwrap()
    };
    let key = Arc::new(key);

    let url = ask_daemon_url();

    test_host(&config, root.clone(), key.clone(), key.id);

    let request = config_to_update_request(url, config);
    Client::default()
        .post(backend.join("/api/internal/daemon/update").unwrap())
        .bearer_auth(code)
        .json(&request)
        .send()
        .unwrap()
        .error_for_status()
        .unwrap();

    println!("Done");
}

fn test_host(config: &DaemonConfig, root: Arc<DaemonRootKey>, key: Arc<DaemonKey>, uuid: Uuid) {
    #[derive(Clone)]
    struct ConnectionTestWrapper;

    #[async_trait]
    impl SetupService for ConnectionTestWrapper {
        async fn connection_test(&self) -> Result<(), ConnectionTestError> {
            Err(ConnectionTestError::AwaitingSetup)
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

pub(crate) fn new(backend_url: Option<Url>, code: Option<String>, skip_mariadb_plugin: bool) {
    let (backend, code) = default(backend_url, code);

    println!("Retrieving daemon root key...");
    let Some((root, rsa)) = read_root_key(backend.clone(), code.clone()).unwrap() else {
        return;
    };
    let root = Arc::new(root);

    let mut mounts = Vec::new();

    println!(
        "\nPlease enter the mariadb connection url.\nIt is important that the database is reliable, meaning it should be installed locally and accessed through a unix domain socket.\nThis Database is shared among servers. {}Ensure that the root user is only accessible through a password.{}\nOn most Linux systems the unix socket for the mariadb server is located at /var/run/mysqld/mysqld.sock\nSyntax: mysql://<user>[:<password>]@<host>/<database>[?socket=<unixSocketPath>]\n<...> = required\n[...] = technically optional, but strongly recommended\n",
        termion::color::Bg(termion::color::Red),
        termion::style::Reset
    );
    let mysql = Text::new("URL: ")
        .with_validator(MySQLValidator)
        .prompt()
        .unwrap();

    let mysql_url = Url::parse(&mysql).unwrap();
    let socket = mysql_url
        .query_pairs()
        .into_iter()
        .find(|(param, _)| param.eq("socket"));
    if let Some((_, socket)) = socket {
        let src = PathBuf::from(socket.to_string());
        let src = src.parent().unwrap();
        mounts.push(MountPoint {
            src: src.to_string_lossy().to_string(),
            dest: "/var/run/mysqld".to_string(),
            mode: "ro".to_string(),
        });
    }

    let config = DaemonConfig {
        database: mysql,
        listeners: HashSet::from([
            SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 3000),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 3000),
        ]),
        backend_url: backend.join("/").unwrap(),
        origin: HeaderValue::from_str(&backend.join("/").unwrap().to_string()).unwrap(),
        tls: None,
        ssh: Some(crate::config::SSHConfig {
            listeners: HashSet::from([
                SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 22),
                SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 22),
            ]),
            host_key: vec![
                crate::config::SSHHostKey::Ed25519 {
                    path: "ed25519.pem".to_string(),
                },
                crate::config::SSHHostKey::Rsa {
                    path: "rsa.pem".to_string(),
                },
            ],
            auth_url: backend.join("/").unwrap(),
            auth_cache_duration: 30 * 1000,
        }),
        storage: HashMap::from_iter([(
            "local".to_string(),
            StorageEntry::Directory {
                path: "./storages".to_string(),
            },
        )]),
        backup: HashMap::from_iter([(
            "local".to_string(),
            crate::config::BackupEntry::Archive {
                path: "./backups".to_string(),
                compression: ArchiveCompression::Zstd {
                    level: CompressionLevel::Best,
                },
                order: Some(0),
            },
        )]),
        network: NetworkConfig::default(),
        mounts,
    };

    let host = ask_daemon_url();

    {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open("config.toml")
            .unwrap();
        file.write_all(toml::to_string(&config).unwrap().as_bytes())
            .unwrap();
        file.flush().unwrap();
    }

    std::process::Command::new("vim")
        .stdout(Stdio::inherit())
        .stdin(Stdio::inherit())
        .stderr(Stdio::inherit())
        .arg("config.toml")
        .spawn()
        .unwrap()
        .wait()
        .unwrap();

    let config: DaemonConfig = {
        let mut file = File::open("config.toml").unwrap();
        let mut str = String::new();
        file.read_to_string(&mut str).unwrap();
        toml::from_str(&str).unwrap()
    };

    let keypair = crate::common::key::rcgen::KeyPair::generate_for(if rsa {
        &PKCS_RSA_SHA256
    } else {
        &PKCS_ECDSA_P256_SHA256
    })
    .unwrap();

    let csr = DaemonKey::new_request(&keypair).unwrap();
    let temp_key = Arc::new(DaemonKey::new_temp(&keypair).unwrap());

    test_host(&config, root.clone(), temp_key.clone(), Uuid::nil());

    let request =
        config_to_register_request(host, config, &temp_key.cert[0], &csr, skip_mariadb_plugin);
    let response = Client::default()
        .post(backend.join("/api/internal/daemon/register").unwrap())
        .bearer_auth(code)
        .json(&request)
        .send()
        .unwrap();
    let response = response.error_for_status().unwrap();
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

#[inline(always)]
fn ask_daemon_url() -> Url {
    println!("\nOn what url is the daemon publicly reachable?");
    Url::parse(
        &Text::new("URL: ")
            .with_initial_value(&format!("https://{}:3000", hostname()))
            .with_validator(URLValidator::<true>)
            .prompt()
            .unwrap(),
    )
    .unwrap()
}

fn ask_ssh(config: &DaemonConfig) -> Option<HostAndPort> {
    if config.ssh.is_some() {
        println!("\nOn what ssh addr is the daemon publicly reachable?");
        Some(
            parse_addr(
                Text::new("Addr: ")
                    .with_initial_value(&format!("{}:22", hostname()))
                    .with_validator(HostPortValidator)
                    .prompt()
                    .unwrap(),
            )
            .unwrap(),
        )
        .map(|(host, port)| HostAndPort {
            host,
            port: port.unwrap_or(22),
        })
    } else {
        None
    }
}

#[inline(always)]
fn config_to_register_request(
    url: Url,
    config: DaemonConfig,
    cert: &CertificateDer<'static>,
    csr: &CertificateSigningRequestDer<'static>,
    no_plugin: bool,
) -> NodeRegistrationRequest {
    let ssh_host = ask_ssh(&config);

    if !no_plugin {
        let mysql2 = config.database.clone();
        Handle::current()
            .block_on(async {
                let mut conn = MySqlConnection::connect(&mysql2).await?;

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
        base_url: url.clone(),
        ssh: ssh_host,
        cert: cert.to_vec(),
        csr: csr.to_vec(),
        storages: config
            .storage
            .into_iter()
            .map(|(name, storage)| {
                (
                    name,
                    match storage {
                        StorageEntry::Directory { .. } => ServerStorage::Directory,
                        #[cfg(feature = "btrfs")]
                        StorageEntry::Btrfs { group, .. } => ServerStorage::Btrfs(group),
                    },
                )
            })
            .collect(),
        backups: config
            .backup
            .into_iter()
            .map(|(name, storage)| {
                (
                    name,
                    match storage {
                        BackupEntry::Archive { .. } => BackupStorage::Archive,
                        #[cfg(feature = "btrfs")]
                        BackupEntry::Btrfs { group, .. } => BackupStorage::Btrfs(group),
                    },
                )
            })
            .collect(),
        default_bind: "0.0.0.0".to_string(),
        default_host: url.host_str().unwrap().to_string(),
    }
}

#[inline(always)]
fn config_to_update_request(url: Url, config: DaemonConfig) -> NodeUpdateRequest {
    let ssh_host = ask_ssh(&config);

    NodeUpdateRequest {
        base_url: url.clone(),
        ssh: ssh_host,
        storages: config
            .storage
            .into_iter()
            .map(|(name, storage)| {
                (
                    name,
                    match storage {
                        StorageEntry::Directory { .. } => ServerStorage::Directory,
                        #[cfg(feature = "btrfs")]
                        StorageEntry::Btrfs { group, .. } => ServerStorage::Btrfs(group),
                    },
                )
            })
            .collect(),
        backups: config
            .backup
            .into_iter()
            .map(|(name, storage)| {
                (
                    name,
                    match storage {
                        BackupEntry::Archive { .. } => BackupStorage::Archive,
                        #[cfg(feature = "btrfs")]
                        BackupEntry::Btrfs { group, .. } => BackupStorage::Btrfs(group),
                    },
                )
            })
            .collect(),
    }
}

fn hostname() -> String {
    File::open("/proc/sys/kernel/hostname")
        .map(|mut file| {
            let mut str = String::new();
            let _ = file.read_to_string(&mut str);
            str.trim().to_string()
        })
        .unwrap_or_default()
}

fn read_root_key(url: Url, code: String) -> Result<Option<(DaemonRootKey, bool)>, anyhow::Error> {
    let response = Client::default()
        .get(url.join("/api/internal/daemon/root_key").unwrap())
        .bearer_auth(code)
        .send()?;
    if response.status() == axum::http::StatusCode::UNAUTHORIZED {
        println!("Code is invalid");
        return Ok(None);
    }
    let rsa = response.headers().contains_key("X-Prefer-RSA-Key");
    let str: String = response.json().map_err(io::Error::other)?;
    Ok(Some((DaemonRootKey::parse(str)?.0, rsa)))
}

impl StringValidator for MySQLValidator {
    fn validate(&self, input: &str) -> Result<Validation, CustomUserError> {
        Ok(Handle::current()
            .block_on(MySqlConnection::connect(input))
            .map(|_| Validation::Valid)
            .unwrap_or_else(|err| {
                Validation::Invalid(ErrorMessage::Custom(format!("Error: {:?}", err)))
            }))
    }
}

impl<const SECURE: bool> StringValidator for URLValidator<SECURE> {
    fn validate(&self, input: &str) -> Result<Validation, CustomUserError> {
        Ok(Url::parse(input)
            .map(|url| {
                if !url.has_host() {
                    return Validation::Invalid(ErrorMessage::Default);
                }
                if (url.scheme() == "http" && !SECURE) || url.scheme() == "https" {
                    Validation::Valid
                } else {
                    Validation::Invalid(ErrorMessage::Default)
                }
            })
            .unwrap_or(Validation::Invalid(ErrorMessage::Default)))
    }
}

fn parse_addr(input: impl Into<String>) -> Option<(Host, Option<u16>)> {
    let input = input.into();
    let (host, port): (&str, _) = match input.rsplit_once(":") {
        None => (&input, None),
        Some((host, port)) => (host, Some(port)),
    };
    let Ok(host) = Host::parse(host) else {
        return None;
    };
    let port = port.map(u16::from_str);
    if let Some(Err(_)) = port {
        return None;
    }
    Some((host, port.map(Result::unwrap)))
}

impl StringValidator for HostPortValidator {
    fn validate(&self, input: &str) -> Result<Validation, CustomUserError> {
        Ok(
            parse_addr(input).map_or(Validation::Invalid(ErrorMessage::Default), |_| {
                Validation::Valid
            }),
        )
    }
}

impl StringValidator for RegexValidator {
    fn validate(&self, input: &str) -> Result<Validation, CustomUserError> {
        Ok(if self.0.is_match(input) {
            Validation::Valid
        } else {
            Validation::Invalid(ErrorMessage::Default)
        })
    }
}

#[derive(Copy, Clone)]
struct VecAutoComplete(&'static [&'static str]);

impl Autocomplete for VecAutoComplete {
    fn get_suggestions(&mut self, _input: &str) -> Result<Vec<String>, CustomUserError> {
        Ok(self.0.iter().map(|str| String::from(*str)).collect())
    }

    fn get_completion(
        &mut self,
        _input: &str,
        highlighted_suggestion: Option<String>,
    ) -> Result<Replacement, CustomUserError> {
        Ok(highlighted_suggestion)
    }
}
