use crate::config::{ArchiveCompression, CompressionLevel};
use crate::config::{BackupEntry, DaemonConfig, MountPoint, NetworkConfig, StorageEntry};
use async_trait::async_trait;
use axum::Extension;
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use daemon::internal::middleware;
use daemon_common::framework_router;
use daemon_common::payload::node::{BackupStorage, HostAndPort, NodeRegistrationRequest, ServerStorage};
use daemon_common::service::setup::SetupService;
use daemon_common::service::setup::error::ConnectionTestError;
use ed25519_dalek::VerifyingKey;
use inquire::autocompletion::Replacement;
use inquire::validator::{ErrorMessage, StringValidator, Validation};
use inquire::{Autocomplete, CustomUserError, Text};
use rand::TryRngCore;
use rand::rngs::OsRng;
use regex::Regex;
use reqwest::{Client, Url};
use sqlx::{Connection, MySqlConnection, QueryBuilder, query_scalar};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::process::{Stdio, exit};
use std::str::FromStr;
use std::time::Duration;
use std::{io, thread};
use std::sync::Arc;
use tokio::runtime::Handle;
use tracing::info;
use url::Host;
use utoipa_axum::router::OpenApiRouter;

#[derive(Copy, Clone)]
struct MySQLValidator;

#[derive(Copy, Clone)]
struct URLValidator;

#[derive(Copy, Clone)]
struct HostPortValidator;

#[derive(Clone)]
struct RegexValidator(Regex);

#[derive(Clone)]
struct LengthValidator(RangeInclusive<usize>);

pub(crate) fn setup(backend_url: Option<Url>, code: Option<String>, skip_mariadb_plugin: bool) {
    info!("Starting Setup Wizard...");

    match File::open("config.toml") {
        Ok(file) => {
            println!(
                "\nThere already is an existing configuration for the daemon? Do you want to apply config updates or start a new setup?"
            );
            if Text::new(" ")
                .with_validator(RegexValidator(Regex::new("Update|New").unwrap()))
                .with_autocomplete(VecAutoComplete(&["Update", "New"]))
                .prompt()
                .unwrap()
                .eq("New")
            {
                new(backend_url, code, skip_mariadb_plugin)
            } else {
                update(file, backend_url, code, skip_mariadb_plugin)
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
                .with_validator(URLValidator)
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

pub(crate) fn update(
    mut file: File,
    backend_url: Option<Url>,
    code: Option<String>,
    skip_mariadb_plugin: bool,
) {
    let (backend, code) = default(backend_url, code);
    let config: DaemonConfig = {
        let mut str = String::new();
        file.read_to_string(&mut str).unwrap();
        drop(file);
        toml::from_str(&str).unwrap()
    };

    let url = ask_daemon_url();

    test_host(&config);

    let request = config_to_request(url, config, skip_mariadb_plugin);
    let response = Handle::current()
        .block_on(
            Client::default()
                .patch(backend.join("/api/internal/node").unwrap())
                .bearer_auth(code)
                .json(&request)
                .send(),
        )
        .unwrap();
    response.error_for_status().unwrap();

    println!("Done");

    exit(0);
}

fn test_host(config: &DaemonConfig) {
    #[derive(Clone)]
    struct ConnectionTestWrapper;

    #[async_trait]
    impl SetupService for ConnectionTestWrapper {
        async fn connection_test(&self) -> Result<(), ConnectionTestError> {
            Err(ConnectionTestError::AwaitingSetup)
        }
    }

    let rustls = config.ssl.as_ref().map(|ssl| {
        tokio::runtime::Handle::current()
            .block_on(ssl.as_rustls())
            .unwrap()
    });

    for listener in config.listeners.clone() {
        let (routes, _) =
            framework_router!(ConnectionTestWrapper ConnectionTestWrapper { SetupService });
        let router = OpenApiRouter::new()
            .merge(routes)
            .layer(axum::middleware::from_fn(middleware))
            .layer(Extension(Arc::new(config.clone())));
        let (router, _) = router.split_for_parts();

        if let Some(rustls) = rustls.clone() {
            tokio::task::spawn(
                axum_server::bind_rustls(listener, rustls).serve(router.into_make_service()),
            );
        } else {
            tokio::task::spawn(axum_server::bind(listener).serve(router.into_make_service()));
        }
    }

    thread::sleep(Duration::from_secs(5));
}

pub(crate) fn new(backend_url: Option<Url>, code: Option<String>, skip_mariadb_plugin: bool) {
    let (backend, code) = default(backend_url, code);

    println!("Retrieving public user key...");
    let user_key = Handle::current()
        .block_on(read_user_key(backend.clone(), code.clone()))
        .unwrap();
    println!("Success");

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

    let mut bytes = [0; 128];
    OsRng.try_fill_bytes(&mut bytes).unwrap();
    let token = BASE64_STANDARD.encode(bytes);

    println!(
        "\nPlease enter the node identification.\nThis is a string for the identification of the node in the backend and for authentication. It should not be visible to the user."
    );
    let identifier = Text::new("Identifier: ")
        .with_validator(RegexValidator(Regex::new("[A-Za-z0-9_\\.-]+").unwrap()))
        .with_validator(LengthValidator(1..=16))
        .prompt()
        .unwrap();

    let config = DaemonConfig {
        database: mysql,
        listeners: HashSet::from([
            SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 3000),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 3000),
        ]),
        token,
        identifier,
        backend_url: backend.join("/").unwrap(),
        user_key,
        origin: backend.join("/").unwrap().to_string(),
        ssl: None,
        ssh: Some(crate::config::SSHConfig {
            listeners: HashSet::from([
                SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 22),
                SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 22),
            ]),
            host_key: vec![
                crate::config::SSHHostKey::ED25519 {
                    path: "ed25519.pem".to_string(),
                },
                crate::config::SSHHostKey::RSA {
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

    test_host(&config);

    let request = config_to_request(host, config, skip_mariadb_plugin);
    let response = Handle::current()
        .block_on(
            Client::default()
                .put(backend.join("/api/internal/node").unwrap())
                .bearer_auth(code)
                .json(&request)
                .send(),
        )
        .unwrap();
    response.error_for_status().unwrap();

    println!("Done");

    exit(0);
}

#[inline(always)]
fn ask_daemon_url() -> Url {
    println!("\nOn what url is the daemon publicly reachable?");
    Url::parse(
        &Text::new("URL: ")
            .with_initial_value(&format!("http://{}:3000", hostname()))
            .with_validator(URLValidator)
            .prompt()
            .unwrap(),
    )
    .unwrap()
}

fn config_to_request(url: Url, config: DaemonConfig, no_plugin: bool) -> NodeRegistrationRequest {
    let ssh_host = if config.ssh.is_some() {
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
    };

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
        identifier: config.identifier,
        token: config.token,
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
        default_bind: "0.0.0.0".to_string(),
        default_host: url.host_str().unwrap().to_string(),
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

async fn read_user_key(url: Url, code: String) -> Result<VerifyingKey, io::Error> {
    let response = reqwest::Client::default()
        .get(url.join("/api/internal/user_key").unwrap())
        .bearer_auth(code)
        .send()
        .await
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
    if response.status() == axum::http::StatusCode::UNAUTHORIZED {
        exit(0);
    }
    let str: String = response
        .json()
        .await
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
    VerifyingKey::try_from(
        BASE64_STANDARD
            .decode(str)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?
            .as_slice(),
    )
    .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
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

impl StringValidator for URLValidator {
    fn validate(&self, input: &str) -> Result<Validation, CustomUserError> {
        Ok(Url::parse(input)
            .map(|url| {
                if !url.has_host() {
                    return Validation::Invalid(ErrorMessage::Default);
                }
                if url.scheme() == "http" || url.scheme() == "https" {
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
        Ok(parse_addr(input)
            .map(|_| Validation::Valid)
            .unwrap_or(Validation::Invalid(ErrorMessage::Default)))
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

impl StringValidator for LengthValidator {
    fn validate(&self, input: &str) -> Result<Validation, CustomUserError> {
        Ok(if self.0.contains(&input.len()) {
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
