// SPDX-License-Identifier: AGPL-3.0-only

pub(crate) mod setup;

use crate::config::DaemonConfig;
use clap::*;
use daemon::*;
use daemon_common::key::{DaemonKey, DaemonRootKey};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::{self, File};
use tokio::io::AsyncReadExt as _;
use tracing::info;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use url::Url;

fn main() {
    let matches = command!()
        .about("hosting-system daemon")
        .args_override_self(true)
        .arg(
            Arg::new("debug")
                .short('d')
                .long("debug")
                .env("DEBUG")
                .help("Enables debug output")
                .required(false)
                .action(ArgAction::SetTrue)
                .default_value(if cfg!(debug_assertions) {
                    "true"
                } else {
                    "false"
                }),
        )
        .arg(
            Arg::new("skip-capability-setup")
                .short(None)
                .long("skip-capability-setup")
                .env("SKIP_CAPABILITY_SETUP")
                .help("Skips the setuid")
                .required(false)
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("skip-mariadb-plugin")
                .short(None)
                .long("skip-mariadb-plugin")
                .env("SKIP_MARIADB_PLUGIN")
                .help("Skips the mariadb plugin install step (Only relevant for setup)")
                .action(ArgAction::SetTrue)
                .required(false),
        )
        .arg(
            Arg::new("setup-url")
                .short(None)
                .long("setup-url")
                .env("SETUP_URL")
                .help("Sets the URL to use for setup (Only relevant for setup)")
                .action(ArgAction::Set)
                .required(false),
        )
        .arg(
            Arg::new("setup-token")
                .short(None)
                .long("setup-token")
                .env("SETUP_TOKEN")
                .help("Sets the token to use for setup (Only relevant for setup)")
                .action(ArgAction::Set)
                .required(false),
        )
        .arg(
            Arg::new("database-url")
                .short(None)
                .long("database-url")
                .env("DATABASE_URL")
                .help("Sets the database URL to use")
                .action(ArgAction::Set)
                .required(true)
                .value_parser(value_parser!(Url))
                .default_value("mysql://root@localhost/daemon"),
        )
        .arg(
            Arg::new("port")
                .short(None)
                .long("port")
                .env("PORT")
                .help("Sets the port to use for the daemon")
                .action(ArgAction::Set)
                .required(false)
                .value_parser(value_parser!(u16))
                .default_value("8080"),
        )
        .arg(
            Arg::new("runtime-dir")
                .short(None)
                .long("runtime-dir")
                .env("DAEMON_RUNTIME_DIR")
                .help("Runtime directory to use. Must be path identical on docker host")
                .action(ArgAction::Set)
                .default_value(daemon_common::DAEMON_DEFAULT_RUNTIME_DIR),
        )
        .get_matches();

    if !matches.get_flag("skip-capability-setup") {
        fsuid_setup();
    }

    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Unable to set aws_lc_sys as default crypto provider");

    let reg = tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::from_level(
            if matches.get_flag("debug") {
                tracing::Level::DEBUG
            } else {
                tracing::Level::INFO
            },
        ))
        .with(tracing_subscriber::fmt::layer());

    #[cfg(not(all(feature = "sentry", not(debug_assertions))))]
    reg.init();

    #[cfg(all(feature = "sentry", not(debug_assertions)))]
    let guard = {
        reg.with(sentry::integrations::tracing::layer()).init();
        let sentry = sentry::init((
            "https://79a1f560730aef34fdc4e4a3f3d9ce92@o4507044844404736.ingest.us.sentry.io/4508313795297280",
            sentry::ClientOptions::new().maybe_release(sentry::release_name!()),
        ));
        if sentry.is_enabled() {
            info!("Enabled Sentry integration");
        }
        sentry
    };

    main0(matches);

    // Ensure it gets dropped, only after the daemon exists
    #[cfg(all(feature = "sentry", not(debug_assertions)))]
    drop(guard);
}

#[tokio::main]
async fn main0(matches: ArgMatches) {
    let config_needs_setup = match fs::read_to_string("config.toml").await {
        Ok(contents) => contents.trim().is_empty(),
        Err(err) => {
            assert_eq!(
                err.kind(),
                ErrorKind::NotFound,
                "Unable to read config file: {err}"
            );
            true
        }
    };

    let database_url = matches
        .get_one::<Url>("database-url")
        .expect("database-url is required")
        .clone();

    if config_needs_setup {
        tokio::task::spawn_blocking(move || {
            let skip_mariadb_plugin = matches.get_flag("skip-mariadb-plugin");

            let setup_url = matches
                .get_one::<String>("setup-url")
                .expect("setup-url is required for setup");

            let setup_token = matches
                .get_one::<String>("setup-token")
                .expect("setup-token is required for setup");

            let port = *matches
                .get_one::<u16>("port")
                .expect("port is required for setup");

            setup::setup(
                skip_mariadb_plugin,
                setup_url,
                setup_token,
                &database_url,
                port,
            );
        })
        .await
        .expect("Setup failed");
    } else {
        let runtime_dir = matches
            .get_one::<String>("runtime-dir")
            .expect("runtime-dir is required for operation");

        run(database_url, PathBuf::from(runtime_dir)).await;
    }
}

async fn run(database_url: Url, runtime_dir: PathBuf) {
    let mut config_buffer = String::new();
    File::open("config.toml")
        .await
        .expect("Failed to open config file")
        .read_to_string(&mut config_buffer)
        .await
        .expect("Unable to read config file");
    let mut config: DaemonConfig =
        toml::from_str(&config_buffer).expect("Failed to parse config file");

    if config.database.is_empty() {
        config.database = database_url.to_string();
    }

    let mut root = Vec::new();
    let mut key = Vec::new();
    {
        let mut root_file = File::open("daemon_root.pem")
            .await
            .expect("Unable to open daemon root key (daemon_root.pem)");
        root_file
            .read_to_end(&mut root)
            .await
            .expect("Unable to read daemon root key (daemon_root.pem)");

        let mut key_file = File::open("daemon.pem")
            .await
            .expect("Unable to open daemon key (daemon.pem)");
        key_file
            .read_to_end(&mut key)
            .await
            .expect("Unable to read daemon key (daemon.pem)");
    };
    let root = DaemonRootKey::parse(&root)
        .expect("Unable to parse daemon root key (daemon_root.pem)")
        .0;
    let key = DaemonKey::parse(&root, &key).expect("Unable to parse daemon key (daemon.pem)");

    info!("Starting Daemon on version {GIT_HASH}");

    let daemon = Daemon::init(config, Arc::new(root), Arc::new(key), runtime_dir)
        .await
        .expect("Unable to initialize daemon");

    info!("Starting Webserver...");

    daemon
        .serve_all()
        .await
        .expect("Unable to serve all services");

    daemon.hook_signals();

    daemon.await_shutdown().await;
}

#[expect(clippy::allow_attributes, reason = "platform depending")]
#[allow(
    clippy::missing_const_for_fn,
    reason = "function performs non-const Linux capability setup behind cfg"
)]
fn fsuid_setup() {
    #[cfg(target_os = "linux")]
    {
        use nix::unistd::{Gid, Uid};
        use std::os::unix::fs::MetadataExt as _;
        if std::fs::metadata("/proc/self/")
            .expect("Unable to retrieve uid of /proc/self/")
            .uid()
            != 0
        {
            return;
        }
        capctl::prctl::set_keepcaps(true).expect("Unable to keep capabilities after setuid");
        let mut capstate =
            capctl::CapState::get_current().expect("Unable to retrieve current capabilities");
        capstate.permitted.add(capctl::Cap::DAC_OVERRIDE);

        nix::unistd::setfsuid(Uid::from_raw(u32::from(common::CONTAINER_DEFAULT_UID)));
        nix::unistd::setfsgid(Gid::from_raw(u32::from(common::CONTAINER_DEFAULT_UID)));

        let mut capstate =
            capctl::CapState::get_current().expect("Unable to retrieve current capabilities");
        capstate.effective.add(capctl::Cap::DAC_OVERRIDE);
        capstate
            .set_current()
            .expect("Unable to set effective capabilities");
    }
}
