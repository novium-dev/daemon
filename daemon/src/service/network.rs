// SPDX-License-Identifier: AGPL-3.0-only
use crate::network::Network;
use crate::{Daemon, DaemonExtra};
use async_trait::async_trait;
use daemon_common::service::network::NetworkService;
use daemon_common::service::network::network_service_error::{
    CreateNetworkError, DeleteNetworkError,
};
use sqlx::Connection as _;
use std::sync::Arc;
use tracing::error;

#[async_trait]
impl<T: DaemonExtra> NetworkService for Daemon<T> {
    async fn create_network(
        &self,
        ipv4: Option<u16>,
        ipv6: u16,
    ) -> Result<u32, CreateNetworkError> {
        let mut conn = self.conn.acquire().await?;
        let docker = Arc::clone(&self.docker);
        tokio::task::spawn(async move {
            let network = conn
                .transaction(move |tx| {
                    Box::pin(async move {
                        Network::create::<CreateNetworkError>(&docker, tx, ipv4, ipv6).await
                    })
                })
                .await
                .inspect_err(|err| error!("Failed to create network: {:?}", err))?;
            Ok(network.id)
        })
        .await
        .expect("Network create task panicked")
    }

    async fn delete_network(&self, network_id: u32) -> Result<(), DeleteNetworkError> {
        let Some(network) = Network::resolve::<DeleteNetworkError>(&*self.conn, network_id).await?
        else {
            return Err(DeleteNetworkError::NetworkNotFound);
        };
        let mut conn = self.conn.acquire().await?;
        let docker = Arc::clone(&self.docker);
        tokio::task::spawn(async move {
            conn.transaction(move |tx| {
                Box::pin(async move { network.delete::<DeleteNetworkError>(&docker, tx).await })
            })
            .await?;
            Ok::<_, DeleteNetworkError>(())
        })
        .await
        .expect("Network deletion task panicked")?;

        Ok(())
    }
}
