// SPDX-License-Identifier: AGPL-3.0-only
use crate::{Daemon, DaemonExtra};
use axum::Router;
use axum::extract::{ConnectInfo, State, connect_info};
use axum::http::StatusCode;
use axum::routing::get;
use axum::serve::IncomingStream;
use core::sync::atomic::Ordering;
use rustix::fs::Mode;
use rustix::process::umask;
use std::io;
use tokio::net::{UnixListener, unix::UCred};
use tracing::{error, warn};

#[derive(Clone, Debug)]
struct UdsConnectInfo {
    peer_cred: Option<UCred>,
}

impl connect_info::Connected<IncomingStream<'_, UnixListener>> for UdsConnectInfo {
    fn connect_info(stream: IncomingStream<'_, UnixListener>) -> Self {
        let peer_cred = stream.io().peer_cred().ok();
        Self { peer_cred }
    }
}

pub async fn run(daemon: Daemon<impl DaemonExtra>) -> Result<(), io::Error> {
    let path = daemon.runtime_dir.join("control.sock");
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

    match state.servers.lock().await.get(&container) {
        None => {
            warn!(
                "Mark restart got called from a server that has no server data allocated (the server should be offline)"
            );
        }
        Some(server) => {
            server.restart.store(true, Ordering::Relaxed);
        }
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use std::collections::{HashMap, HashSet};

    use daemon_common::service::{
        files::FilesService as _,
        server::{ServerBuildImage, ServerBuildPayload, ServerService as _},
    };

    use crate::test::{create_dummy_server, test, wait_server_stopped_delayed};

    #[test]
    fn mark_restart() {
        test(|ctx| async move {
            let server_id = create_dummy_server(&ctx.daemon)
                .await
                .expect("failed to create dummy server");

            ctx.daemon
                .update_server(
                    server_id,
                    None,
                    Some(ServerBuildPayload {
                        backup_exclude: HashSet::new(),
                        cap_add: Vec::new(),
                        cap_drop: Vec::new(),
                        environment: HashMap::new(),
                        files_write: HashMap::new(),
                        uid: None,
                        gid: None,
                        stop_command: None,
                        stop_timeout: 30 * 1000,
                        image: ServerBuildImage {
                            args: HashMap::new(),
                            base: None,
                            default_tag: Some("ctlsock-test-r1".to_owned()),
                            dockerfile: "FROM alpine/curl:latest\nENTRYPOINT [\"/bin/sh\", \"-c\", \"(cat restart && touch success) || (curl --unix-socket /control.sock http://localhost/mark_restart && touch restart)\"]".to_owned(),
                            parents: Vec::new(),
                            tag: None,
                        },
                    }),
                    None,
                    None,
                    None,
                )
                .await
                .expect("failed to update server");

            ctx.daemon
                .start_server(server_id)
                .await
                .expect("failed to start server");

            wait_server_stopped_delayed(&ctx.daemon, server_id, 120).await;
            wait_server_stopped_delayed(&ctx.daemon, server_id, 30).await;

            ctx.daemon
                .server_stat_file(server_id, "success".to_owned())
                .await
                .expect("failed to stat success file");
        });
    }
}
