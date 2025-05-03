include!("lib.rs");

pub(crate) mod setup;

use clap::*;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tracing::{error, info};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

fn main() {
    let matches = command!()
        .about("hosting-system daemon")
        .args_override_self(true)
        .subcommand(
            Command::new("setup")
                .args_override_self(true)
                .short_flag('s')
                .long_flag("setup")
                .about("Start the setup wizard")
                .arg(
                    Arg::new("backend")
                        .short('b')
                        .long("backend")
                        .env("BACKEND_URL")
                        .help("The public accessible backend url")
                        .action(ArgAction::Set)
                        .required(false),
                )
                .arg(
                    Arg::new("code")
                        .short('c')
                        .long("code")
                        .env("REGISTRATION_CODE")
                        .help("The registration code")
                        .action(ArgAction::Set)
                        .required(false),
                )
                .arg(
                    Arg::new("skip-mariadb-plugin")
                        .short(None)
                        .long("skip-mariadb-plugin")
                        .env("SKIP_MARIADB_PLUGIN")
                        .help("Skips the mariadb plugin install step")
                        .action(ArgAction::SetTrue)
                        .required(false),
                ),
        )
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
        .get_matches();

    if !matches.get_flag("skip-capability-setup") {
        cap_setup();
    }

    let reg = tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::from_level(
            if matches.get_flag("debug") {
                tracing::Level::DEBUG
            } else {
                tracing::Level::INFO
            },
        ))
        .with(tracing_subscriber::fmt::layer());

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Unable to set ring as default crypto provider");

    #[cfg(not(all(feature = "sentry", not(debug_assertions))))]
    reg.init();

    #[cfg(all(feature = "sentry", not(debug_assertions)))]
    let guard = {
        reg.with(sentry::integrations::tracing::layer()).init();
        let sentry = sentry::init((
            "https://79a1f560730aef34fdc4e4a3f3d9ce92@o4507044844404736.ingest.us.sentry.io/4508313795297280",
            sentry::ClientOptions {
                release: sentry::release_name!(),
                ..Default::default()
            },
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
    match matches.subcommand() {
        Some(("setup", _)) => {
            tokio::task::spawn_blocking(move || {
                let setup_matches = matches.subcommand_matches("setup").unwrap();
                let backend = setup_matches.get_one("backend");
                let code = setup_matches.get_one("code");
                let skip_mariadb_plugin = setup_matches.get_flag("skip-mariadb-plugin");
                setup::setup(backend.cloned(), code.cloned(), skip_mariadb_plugin);
            })
            .await
            .expect("Setup wizard failed");
        }
        None => {
            run().await;
        }
        _ => unreachable!(),
    }
}

async fn run() {
    let mut config_buffer = String::new();
    File::open("config.toml")
        .await
        .inspect_err(|_err| error!("Failed to open config file"))
        .unwrap()
        .read_to_string(&mut config_buffer)
        .await
        .unwrap();
    let config: DaemonConfig = toml::from_str(&config_buffer)
        .inspect_err(|_err| error!("Failed to read config file"))
        .unwrap();

    info!("Starting Daemon on version {}", GIT_HASH);

    let daemon = Daemon::init(config).await.unwrap();

    info!("Starting Webserver...");

    daemon.serve_http().await.unwrap();

    daemon.serve_ssh().await.unwrap();

    daemon.hook_signals();

    daemon.await_shutdown().await;
}

fn cap_setup() {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        if std::fs::metadata("/proc/self/").unwrap().uid() != 0 {
            return;
        }
        capctl::prctl::set_keepcaps(true).expect("Unable to keep capabilities after setuid");
        let mut capstate =
            capctl::CapState::get_current().expect("Unable to retrieve current capabilities");
        capstate.permitted.add(capctl::Cap::NET_BIND_SERVICE);
        capstate.permitted.add(capctl::Cap::DAC_OVERRIDE);
        capstate.permitted.add(capctl::Cap::FOWNER);
        capstate.permitted.add(capctl::Cap::CHOWN);
        capstate.permitted.add(capctl::Cap::SYS_CHROOT);
        capstate.permitted.add(capctl::Cap::SYS_ADMIN);
        capstate
            .set_current()
            .expect("Unable to set permitted capabilities");

        capctl::cap_set_ids(
            Some(crate::common::CONTAINER_DEFAULT_UID as u32),
            Some(crate::common::CONTAINER_DEFAULT_GID as u32),
            None,
        )
        .expect("Unable to setuid current process");

        let mut capstate = capctl::CapState::get_current().unwrap();
        capstate.effective.add(capctl::Cap::NET_BIND_SERVICE);
        capstate.effective.add(capctl::Cap::DAC_OVERRIDE);
        capstate.effective.add(capctl::Cap::FOWNER);
        capstate.effective.add(capctl::Cap::CHOWN);
        capstate.effective.add(capctl::Cap::SYS_CHROOT);
        capstate.effective.add(capctl::Cap::SYS_ADMIN);
        capstate
            .set_current()
            .expect("Unable to set effective capabilities");
    }
}
