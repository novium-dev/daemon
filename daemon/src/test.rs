#![expect(clippy::panic)]

use daemon_common::service::server::{
    ServerBuildImage, ServerBuildPayload, ServerNetworkPayload, ServerResourcePayload,
    ServerService as _, server_service_error::CreateServerError,
};
use ed25519_dalek::SigningKey;
use rustls::pki_types::PrivateKeyDer;
use sqlx::MySql;
use sqlx::migrate::MigrateDatabase as _;
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv6Addr, SocketAddr},
    panic::Location,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tracing::{Instrument as _, info_span};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use url::Url;
use uuid::Uuid;
use zelus_keys::{
    DaemonKey, DaemonRootKey,
    rcgen::{CertificateSigningRequestParams, KeyPair},
};

use crate::{
    Daemon, WithUserKey,
    config::{
        ArchiveCompression, BackupEntry, CompressionLevel, DaemonConfig, LimitConfig,
        NetworkConfig, SSHConfig, SSHHostKey, StorageEntry,
    },
    fsuid_setup,
};

pub(crate) struct Context {
    pub(crate) rt: tokio::runtime::Runtime,
    pub(crate) daemon: Daemon<WithUserKey>,
}

const PORT_HTTPS: u16 = 30033;
const PORT_SSH: u16 = 30034;
const PORT_BACKEND_DUMMY: u16 = 30035;

static CONTEXT: OnceLock<Context> = OnceLock::new();

#[track_caller]
pub(crate) fn test<F: FnOnce(&'static Context) -> Fut, Fut: Future<Output = ()> + Send>(func: F) {
    let location = Location::caller();

    fsuid_setup();

    let ctx = CONTEXT.get_or_init(|| {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .expect("failed to install aws_lc_rs crypto provider");

        tracing_subscriber::registry()
            .with(tracing_subscriber::filter::LevelFilter::from_level(
                if std::env::var("DEBUG")
                    .is_ok_and(|value| value.eq_ignore_ascii_case("true") || value.eq("1"))
                {
                    tracing::Level::DEBUG
                } else {
                    tracing::Level::INFO
                },
            ))
            .with(tracing_subscriber::fmt::layer())
            .init();

        let ca = KeyPair::generate().expect("failed to generate daemon root keypair");

        let mut rng = rand::rng();

        let user_key = SigningKey::generate(&mut rng);
        let public_user_key = user_key.verifying_key();

        let root = DaemonRootKey::new(&ca, "Test CA", Some(public_user_key))
            .expect("failed to create daemon root key");

        let node_id = Uuid::new_v4();

        let node_keypair = KeyPair::generate().expect("failed to generate daemon keypair");
        let node_req = DaemonKey::new_request(&node_keypair, "node")
            .expect("failed to create daemon key sign request");

        let node_req = CertificateSigningRequestParams::from_der(&node_req)
            .expect("failed to parse cert sign params");

        let node_cert = root
            .sign_daemon_key(&ca, node_req.public_key, "node", node_id)
            .expect("failed to sign daemon key");
        let node_privkey = PrivateKeyDer::try_from(node_keypair.serialize_der())
            .expect("failed to parse daemon privkey der");

        let node_key = DaemonKey::new(&root, node_cert, node_privkey, "node")
            .expect("failed to create daemon key");

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime");

        let backend_url = Url::parse(&format!("https://localhost:{PORT_BACKEND_DUMMY}"))
            .expect("failed to parse dummy backend url");

        let config = DaemonConfig {
            backend_url: backend_url.clone(),
            backup: HashMap::from([(
                "local".to_owned(),
                BackupEntry::Archive {
                    compression: ArchiveCompression::Zstd {
                        level: CompressionLevel::Default,
                    },
                    path: format!("/tmp/daemon_test_{node_id}/backup"),
                    order: Some(0),
                },
            )]),
            storage: HashMap::from([(
                "local".to_owned(),
                StorageEntry::Directory {
                    path: format!("/tmp/daemon_test_{node_id}/server"),
                },
            )]),
            database: format!("{}_test", env!("DATABASE_URL")),
            limit: LimitConfig::default(),
            metrics: None,
            listeners: HashSet::from([SocketAddr::new(
                IpAddr::V6(Ipv6Addr::LOCALHOST),
                PORT_HTTPS,
            )]),
            mounts: vec![],
            network: NetworkConfig::default(),
            tls: None,
            ssh: Some(SSHConfig {
                listeners: HashSet::from([SocketAddr::new(
                    IpAddr::V6(Ipv6Addr::LOCALHOST),
                    PORT_SSH,
                )]),
                auth_url: backend_url,
                host_key: vec![
                    SSHHostKey::Ed25519 {
                        path: format!("/tmp/daemon_test_{node_id}/ed25519.pem"),
                    },
                    SSHHostKey::Rsa {
                        path: format!("/tmp/daemon_test_{node_id}/rsa.pem"),
                    },
                ],
                auth_cache_duration: 30 * 1000,
            }),
            shutdown_stop_servers: true,
            origins: Vec::new(),
            public_url: Url::parse(&format!("https://localhost:{PORT_HTTPS}"))
                .expect("failed to parse base url"),
        };

        std::fs::create_dir_all(format!("/tmp/daemon_test_{node_id}"))
            .expect("failed to create tmp directory");

        let runtime_dir = PathBuf::from(format!("/run/daemon_test/{node_id}"));

        let daemon = rt.block_on(async move {
            if MySql::database_exists(&config.database)
                .await
                .expect("failed to check if test database exists")
            {
                MySql::drop_database(&config.database)
                    .await
                    .expect("failed to drop test database");
            }
            MySql::create_database(&config.database)
                .await
                .expect("failed to create test database");

            Daemon::init_embedded(
                config,
                Arc::new(root),
                Arc::new(node_key),
                runtime_dir,
                user_key,
            )
            .await
            .expect("failed to init daemon")
        });

        Context { rt, daemon }
    });
    ctx.rt
        .block_on(func(ctx).instrument(info_span!("test", location = location.to_string())));
}

#[test]
fn init() {
    test(|_ctx| std::future::ready(()));
}

pub(crate) fn create_dummy_server(
    daemon: &Daemon<WithUserKey>,
) -> impl Future<Output = Result<u32, CreateServerError>> + Send {
    daemon.create_server(
        "local".to_owned(),
        ServerResourcePayload {
            cpu: Some(1.0f32),
            ram: Some(2048),
            disk: None,
            swap: Some(4096),
        },
        ServerBuildPayload {
            backup_exclude: HashSet::new(),
            cap_add: Vec::new(),
            cap_drop: vec!["DAC_OVERRIDE".to_owned()],
            environment: HashMap::new(),
            files_write: HashMap::new(),
            uid: None,
            gid: None,
            image: ServerBuildImage {
                base: None,
                args: HashMap::new(),
                default_tag: Some("test-dummy-r1".to_owned()),
                dockerfile:
                    "FROM debian:latest\nENTRYPOINT [\"/bin/sh\", \"-c\", \"echo Hello World\"]"
                        .to_owned(),
                parents: Vec::new(),
                tag: None,
            },
            stop_command: None,
            stop_timeout: 30,
        },
        Vec::new(),
        ServerNetworkPayload {
            internet_access: false,
            networks: Vec::new(),
        },
        String::new(),
    )
}

pub(crate) async fn wait_server_stopped_delayed(
    daemon: &Daemon<WithUserKey>,
    server_id: u32,
    timeout_sec: u32,
) {
    for _ in 0..timeout_sec {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if daemon
            .server_status(server_id)
            .await
            .expect("failed to get server status")
            .state
            .start
        {
            return;
        }
    }
    panic!("Stop timeout");
}
