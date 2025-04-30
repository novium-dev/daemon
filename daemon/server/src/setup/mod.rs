use crate::config::{BackupEntry, DaemonConfig, MountPoint, NetworkConfig, StorageEntry};
use crate::storage::archive::compression::{ArchiveCompression, CompressionLevel};
use axum::Router;
use axum::routing::get;
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use daemon_common::payload::node::{BackupStorage, NodeRegistrationRequest, ServerStorage};
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
use std::net::SocketAddr;
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::process::{Stdio, exit};
use std::time::Duration;
use std::{env, io, thread};
use tokio::runtime::Handle;

#[derive(Copy, Clone)]
struct MySQLValidator;

#[derive(Copy, Clone)]
struct URLValidator;

#[derive(Clone)]
struct RegexValidator(Regex);

#[derive(Clone)]
struct LengthValidator(RangeInclusive<usize>);

pub(crate) fn setup() {
    println!("Daemon Setup");

    #[cfg(not(debug_assertions))]
    {
        use std::os::unix::fs;
        if !std::fs::exists("/var/lib/novium_daemon").unwrap() {
            std::fs::create_dir_all("/var/lib/novium_daemon").unwrap();
        }
        fs::chown(
            "/var/lib/novium_daemon",
            Some(crate::common::CONTAINER_DEFAULT_UID as u32),
            Some(crate::common::CONTAINER_DEFAULT_GID as u32),
        )
        .unwrap();
        env::set_current_dir(PathBuf::from("/var/lib/novium_daemon")).unwrap();

        capctl::prctl::set_keepcaps(true).unwrap();
        let mut capstate = capctl::CapState::get_current().unwrap();
        capstate.permitted.add(capctl::Cap::NET_BIND_SERVICE);
        capstate.permitted.add(capctl::Cap::DAC_OVERRIDE);
        capstate.set_current().unwrap();

        capctl::cap_set_ids(
            Some(crate::common::CONTAINER_DEFAULT_UID as u32),
            Some(crate::common::CONTAINER_DEFAULT_GID as u32),
            None,
        )
        .unwrap();

        let mut capstate = capctl::CapState::get_current().unwrap();
        capstate.effective.add(capctl::Cap::NET_BIND_SERVICE);
        capstate.effective.add(capctl::Cap::DAC_OVERRIDE);
        capstate.set_current().unwrap();
    }

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
                new()
            } else {
                update(file)
            }
        }
        Err(_) => new(),
    }
}

fn default() -> (Url, String) {
    let mut args = env::args().skip(2); // Skip <current_binary_path> setup

    let backend = args.next().unwrap_or_else(|| {
        println!(
            "\nPlease enter the url to the backend.\nThe API should be reachable using the url."
        );
        Text::new("URL: ")
            .with_validator(URLValidator)
            .prompt()
            .unwrap()
    });
    let code = args.next().unwrap_or_else(|| {
        println!("\nPlease enter your registration code");
        Text::new("Code: ").prompt().unwrap()
    });

    (backend.parse().unwrap(), code)
}

pub(crate) fn update(mut file: File) {
    let (backend, code) = default();
    let config: DaemonConfig = {
        let mut str = String::new();
        file.read_to_string(&mut str).unwrap();
        drop(file);
        toml::from_str(&str).unwrap()
    };

    let host = ask_host();

    test_host(&config);

    let request = config_to_request(host, config);
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
    let app: Router<()> = Router::new().fallback(get(dummy));

    let bind = config.bind.parse().unwrap();
    'ssl: {
        #[cfg(feature = "ssl")]
        if config.ssl.enabled {
            tokio::task::spawn(
                axum_server::bind_rustls(
                    bind,
                    Handle::current().block_on(config.ssl.as_rustls()).unwrap(),
                )
                .serve(app.into_make_service()),
            );
            break 'ssl;
        }
        tokio::task::spawn(axum_server::bind(bind).serve(app.into_make_service()));
    }
    thread::sleep(Duration::from_secs(5));
}

pub(crate) fn new() {
    let (backend, code) = default();

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
        bind: "[::]:3000".to_string(),
        token,
        identifier,
        backend_url: backend.join("/").unwrap(),
        user_key,
        origin: backend.join("/").unwrap().to_string(),
        #[cfg(feature = "ssl")]
        ssl: crate::config::SSLConfig {
            enabled: false,
            private_key: None,
            certificate: None,
        },
        #[cfg(feature = "ssh")]
        ssh: crate::config::SSHConfig {
            bind: "[::]:22".to_string(),
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
        },
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

    let host = ask_host();

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

    let request = config_to_request(host, config);
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

async fn dummy() {
    println!("Connection test received");
}

fn ask_host() -> String {
    println!("\nOn what host is the daemon reachable?");
    Text::new("Host: ")
        .with_initial_value(&hostname())
        .prompt()
        .unwrap()
}

fn config_to_request(host: String, config: DaemonConfig) -> NodeRegistrationRequest {
    let mut scheme = "http".to_string();
    #[cfg(feature = "ssl")]
    if config.ssl.enabled {
        scheme = "https".to_string();
    }

    let bind: SocketAddr = config.bind.parse().unwrap();
    #[cfg(feature = "ssh")]
    let ssh_bind: SocketAddr = config.ssh.bind.parse().unwrap();

    let mut url: Url = format!("{}://localhost", scheme).parse().unwrap();
    url.set_host(Some(&host)).unwrap();
    url.set_port(Some(bind.port())).unwrap();

    #[cfg(feature = "database")]
    {
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
        base_url: url.to_string(),
        #[cfg(feature = "ssh")]
        ssh_addr: host.clone(),
        #[cfg(feature = "ssh")]
        ssh_port: ssh_bind.port(),
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
        default_host: host,
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
                if url.scheme() == "http" || url.scheme() == "https" {
                    Validation::Valid
                } else {
                    Validation::Invalid(ErrorMessage::Default)
                }
            })
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
