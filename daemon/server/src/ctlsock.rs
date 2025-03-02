use crate::DaemonState;
use axum::Router;
use axum::extract::{ConnectInfo, State, connect_info};
use axum::http::StatusCode;
use axum::routing::get;
use axum::serve::IncomingStream;
use libc::umask;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{UnixListener, unix::UCred};
use tracing::{error, warn};

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct UdsConnectInfo {
    peer_addr: Arc<tokio::net::unix::SocketAddr>,
    peer_cred: UCred,
}

impl connect_info::Connected<IncomingStream<'_, UnixListener>> for UdsConnectInfo {
    fn connect_info(stream: IncomingStream<'_, UnixListener>) -> Self {
        let peer_addr = stream.io().peer_addr().unwrap();
        let peer_cred = stream.io().peer_cred().unwrap();
        Self {
            peer_addr: Arc::new(peer_addr),
            peer_cred,
        }
    }
}

pub(crate) async fn run(state: DaemonState) {
    let path = PathBuf::from("./control.sock");
    let _ = tokio::fs::remove_file(&path).await; // Ignore any errors

    let old_umask;
    unsafe {
        old_umask = umask(0);
    } // Use mode 0o777 so containers can access it

    let listener = UnixListener::bind(&path).unwrap();

    unsafe {
        umask(old_umask);
    }

    let app = Router::new()
        .route("/mark_restart", get(restart))
        .with_state(state)
        .into_make_service_with_connect_info::<UdsConnectInfo>();

    axum::serve(listener, app).await.unwrap();
}

async fn restart(
    State(state): State<DaemonState>,
    ConnectInfo(info): ConnectInfo<UdsConnectInfo>,
) -> Result<(), StatusCode> {
    let Some(pid) = info.peer_cred.pid() else {
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
            let server = server.clone();
            drop(servers);
            server.lock().await.restart = true;
        }
    }

    Ok(())
}
