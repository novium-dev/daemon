// SPDX-License-Identifier: AGPL-3.0-only
use crate::ServerData;
use crate::server::FetchedServer;
use crate::server::stop::ServerStopError;
use crate::sync::task::AnyTaskRwLockReadGuard;
use bollard::Docker;
use bollard::container::LogOutput;
use bollard::exec::{CreateExecOptions, StartExecResults};
use cap_async_std::async_std::prelude::Stream;
use core::pin::Pin;
use daemon_common::service::server::task::ServerTask;
use tokio::io::AsyncWrite;

impl FetchedServer {
    pub(crate) async fn exec(
        &self,
        docker: &Docker,
        cmd: Vec<String>,
        server_data: &ServerData,
        lock: &impl AnyTaskRwLockReadGuard<ServerTask, ()>,
        terminal: bool,
    ) -> Result<
        Option<(
            String,
            Pin<Box<dyn Stream<Item = Result<LogOutput, bollard::errors::Error>> + Send>>,
            Pin<Box<dyn AsyncWrite + Send>>,
        )>,
        ServerStopError,
    > {
        if !Self::state(self.id, &self.locks, docker, server_data)
            .await
            .map_err(ServerStopError::Docker)?
            .stop()
        {
            return Err(ServerStopError::NotRunning);
        }
        let exec = docker
            .create_exec(
                &Self::container_name(self.id),
                CreateExecOptions {
                    cmd: Some(cmd),
                    tty: Some(terminal),
                    attach_stdout: Some(terminal),
                    attach_stderr: Some(terminal),
                    attach_stdin: Some(terminal),
                    ..CreateExecOptions::default()
                },
            )
            .await
            .map_err(ServerStopError::Docker)?;
        let start = docker
            .start_exec(&exec.id, None)
            .await
            .map_err(ServerStopError::Docker)?;

        let _ = lock;

        match start {
            StartExecResults::Attached { output, input } => Ok(Some((exec.id, output, input))),
            StartExecResults::Detached => Ok(None),
        }
    }
}
