// SPDX-License-Identifier: AGPL-3.0-only
use crate::ServerData;
use crate::server::FetchedServer;
use crate::sync::task::AnyTaskRwLockReadGuard;
use bollard::Docker;
use bollard::query_parameters::{KillContainerOptions, StopContainerOptions};
use core::error::Error;
use core::fmt::{Debug, Display, Formatter};
use core::sync::atomic::Ordering;
use daemon_common::service::server::task::ServerTask;
use tokio::io::AsyncWriteExt as _;

impl FetchedServer {
    pub async fn stop(
        &self,
        docker: &Docker,
        server_data: &ServerData,
        lock: &impl AnyTaskRwLockReadGuard<ServerTask, ()>,
    ) -> Result<(), ServerStopError> {
        if !Self::state(self.id, &self.locks, docker, server_data)
            .await
            .map_err(ServerStopError::Docker)?
            .stop()
        {
            return Err(ServerStopError::NotRunning);
        }
        server_data.stopping.store(true, Ordering::Relaxed);
        if let Some(stop_command) = &*server_data.stop_command.lock().await {
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
        Self::update_status(self.id, &self.locks, docker, server_data).await;
        docker
            .stop_container(&Self::container_name(self.id), None::<StopContainerOptions>)
            .await
            .map_err(ServerStopError::Docker)?;

        let _ = lock;

        Ok(())
    }

    pub async fn kill(
        &self,
        docker: &Docker,
        server_data: &ServerData,
        lock: &impl AnyTaskRwLockReadGuard<ServerTask, ()>,
    ) -> Result<(), ServerStopError> {
        if !Self::state(self.id, &self.locks, docker, server_data)
            .await
            .map_err(ServerStopError::Docker)?
            .kill()
        {
            return Err(ServerStopError::NotRunning);
        }
        server_data.stopping.store(true, Ordering::Relaxed);
        docker
            .kill_container(&Self::container_name(self.id), None::<KillContainerOptions>)
            .await
            .map_err(ServerStopError::Docker)?;

        let _ = lock;

        Ok(())
    }

    pub async fn restart(
        &self,
        docker: &Docker,
        server_data: &ServerData,
        lock: &impl AnyTaskRwLockReadGuard<ServerTask, ()>,
    ) -> Result<(), ServerStopError> {
        if !Self::state(self.id, &self.locks, docker, server_data)
            .await
            .map_err(ServerStopError::Docker)?
            .stop()
        {
            return Err(ServerStopError::NotRunning);
        }
        server_data.restart.store(true, Ordering::Relaxed);
        self.stop(docker, server_data, lock).await
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
