use crate::{Daemon, DaemonExtra};
use axum::Router;
use axum::extract::{ConnectInfo, State, connect_info};
use axum::http::StatusCode;
use axum::routing::get;
use axum::serve::IncomingStream;
use rustix::fs::Mode;
use rustix::process::umask;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{UnixListener, unix::UCred};
use tracing::{error, warn};

#[derive(Clone, Debug)]
struct UdsConnectInfo {
    peer_cred: Option<UCred>,
}

impl connect_info::Connected<IncomingStream<'_, UnixListener>> for UdsConnectInfo {
    fn connect_info(stream: IncomingStream<'_, UnixListener>) -> Self {
        let peer_cred = stream.io().peer_cred().ok();
        Self {
            peer_cred,
        }
    }
}

pub async fn run(daemon: Daemon<impl DaemonExtra>) -> Result<(), io::Error> {
    let path = PathBuf::from("./control.sock");
    let _ = tokio::fs::remove_file(&path).await; // Ignore any errors

    // Use mode 0o777 so containers can access it
    let old_umask = umask(Mode::empty());

    let listener = UnixListener::bind(&path)?;

    umask(old_umask);

    let app = Router::new()
        .route("/mark_restart", get(restart))
        .with_state(daemon)
        .into_make_service_with_connect_info::<UdsConnectInfo>();

    // It will never error according to doc?
    axum::serve(listener, app).await
}

async fn restart(
    State(state): State<Daemon<impl DaemonExtra>>,
    ConnectInfo(info): ConnectInfo<UdsConnectInfo>,
) -> Result<(), StatusCode> {
    let Some(peer_cred) = info.peer_cred else {
        error!("Mark restart got called without peer credentials");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let Some(pid) = peer_cred.pid() else {
        error!("Mark restart got called without a pid");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let container = crate::common::utils::identify_container_by_pid(&state.docker, pid)
        .await
        .inspect_err(|err| error!("Failed to identify server: {:?}", err))
        .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;

    let servers = state.servers.lock().await;
    match servers.get(&container) {
        None => {
            warn!(
                "Mark restart got called from a server that has no server data allocated (the server should be offline)"
            );
        }
        Some(server) => {
            let server = Arc::clone(server);
            drop(servers);
            server.lock().await.restart = true;
        }
    }

    Ok(())
}
