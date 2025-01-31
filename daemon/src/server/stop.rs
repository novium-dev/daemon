use crate::server::FetchedServer;
use crate::ServerData;
use bollard::Docker;
use std::error::Error;
use std::fmt::{Debug, Display, Formatter};
use std::ops::DerefMut;
use tokio::io::AsyncWriteExt;
use tokio::sync::OwnedMutexGuard;

impl FetchedServer {
    pub(crate) async fn stop(
        &self,
        docker: &Docker,
        mut server_data: OwnedMutexGuard<ServerData>,
    ) -> Result<(), ServerStopError> {
        if !Self::state(self.id, self.locks.clone(), docker, &server_data)
            .await
            .map_err(ServerStopError::Docker)?
            .stop()
        {
            return Err(ServerStopError::NotRunning);
        }
        if let Some(stop_command) = &server_data.stop_command {
            match server_data.attach_write.lock().await.deref_mut() {
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
        server_data.stopping = true;
        Self::update_status0(self.id, self.locks.clone(), docker, &server_data).await;
        drop(server_data);
        docker
            .stop_container(&Self::container_name(self.id), None)
            .await
            .map_err(ServerStopError::Docker)?;
        Ok(())
    }

    pub(crate) async fn kill(
        &self,
        docker: &Docker,
        server_data: OwnedMutexGuard<ServerData>,
    ) -> Result<(), ServerStopError> {
        if !Self::state(self.id, self.locks.clone(), docker, &server_data)
            .await
            .map_err(ServerStopError::Docker)?
            .kill()
        {
            return Err(ServerStopError::NotRunning);
        }
        drop(server_data);
        docker
            .kill_container::<String>(&Self::container_name(self.id), None)
            .await
            .map_err(ServerStopError::Docker)?;
        Ok(())
    }

    pub(crate) async fn restart(
        &self,
        docker: &Docker,
        mut server_data: OwnedMutexGuard<ServerData>,
    ) -> Result<(), ServerStopError> {
        if !Self::state(self.id, self.locks.clone(), docker, &server_data)
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

#[allow(unused)]
#[derive(Debug)]
pub(crate) enum ServerStopError {
    Docker(bollard::errors::Error),
    NotRunning,
}

impl Display for ServerStopError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let debug_self: &dyn Debug = self; // ensure it's not recursively calling itself
        debug_self.fmt(f)
    }
}

impl Error for ServerStopError {}
