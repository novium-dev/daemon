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
use crate::ServerData;
use crate::server::FetchedServer;
use crate::server::stop::ServerStopError;
use cap_async_std::async_std::prelude::Stream;
use bollard::Docker;
use bollard::container::LogOutput;
use bollard::exec::{CreateExecOptions, StartExecResults};
use core::pin::Pin;
use tokio::io::AsyncWrite;

impl FetchedServer {
    pub(crate) async fn exec(
        &self,
        docker: &Docker,
        cmd: Vec<String>,
        server_data: &ServerData,
        terminal: bool,
    ) -> Result<
        Option<(
            String,
            Pin<Box<dyn Stream<Item = Result<LogOutput, bollard::errors::Error>> + Send>>,
            Pin<Box<dyn AsyncWrite + Send>>,
        )>,
        ServerStopError,
    > {
        if !Self::state(self.id, self.locks.clone(), docker, Some(server_data))
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
        match start {
            StartExecResults::Attached { output, input } => Ok(Some((exec.id, output, input))),
            StartExecResults::Detached => Ok(None),
        }
    }
}
