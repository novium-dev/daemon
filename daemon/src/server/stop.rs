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
use bollard::Docker;
use bollard::query_parameters::{KillContainerOptions, StopContainerOptions};
use core::error::Error;
use core::fmt::{Debug, Display, Formatter};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::OwnedMutexGuard;

impl FetchedServer {
    pub async fn stop(
        &self,
        docker: &Docker,
        mut server_data: OwnedMutexGuard<ServerData>,
    ) -> Result<(), ServerStopError> {
        if !Self::state(self.id, self.locks.clone(), docker, Some(&server_data))
            .await
            .map_err(ServerStopError::Docker)?
            .stop()
        {
            return Err(ServerStopError::NotRunning);
        }
        server_data.stopping = true;
        if let Some(stop_command) = &server_data.stop_command {
            match &mut *server_data.attach_write.lock().await {
                None => {}
                Some(write) => {
                    let mut content = "\x08".repeat(4096);
                    content += stop_command;
                    content += "\n";
                    let _ = write.write_all(content.as_bytes()).await;
                    let _ = write.flush().await;
                }
            }
        }
        Self::update_status0(self.id, self.locks.clone(), docker, &server_data).await;
        drop(server_data);
        docker
            .stop_container(&Self::container_name(self.id), None::<StopContainerOptions>)
            .await
            .map_err(ServerStopError::Docker)?;
        Ok(())
    }

    pub async fn kill(
        &self,
        docker: &Docker,
        server_data: OwnedMutexGuard<ServerData>,
    ) -> Result<(), ServerStopError> {
        if !Self::state(self.id, self.locks.clone(), docker, Some(&server_data))
            .await
            .map_err(ServerStopError::Docker)?
            .kill()
        {
            return Err(ServerStopError::NotRunning);
        }
        drop(server_data);
        docker
            .kill_container(&Self::container_name(self.id), None::<KillContainerOptions>)
            .await
            .map_err(ServerStopError::Docker)?;
        Ok(())
    }

    pub async fn restart(
        &self,
        docker: &Docker,
        mut server_data: OwnedMutexGuard<ServerData>,
    ) -> Result<(), ServerStopError> {
        if !Self::state(self.id, self.locks.clone(), docker, Some(&server_data))
            .await
            .map_err(ServerStopError::Docker)?
            .stop()
        {
            return Err(ServerStopError::NotRunning);
        }
        server_data.restart = true;
        self.stop(docker, server_data).await
    }
}

#[derive(Debug)]
pub enum ServerStopError {
    Docker(bollard::errors::Error),
    NotRunning,
}

impl Display for ServerStopError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> core::fmt::Result {
        let debug_self: &dyn Debug = self; // ensure it's not recursively calling itself
        debug_self.fmt(formatter)
    }
}

impl Error for ServerStopError {}
