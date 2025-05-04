use crate::Daemon;
use crate::network::Network;
use async_trait::async_trait;
use daemon_common::service::network::NetworkService;
use daemon_common::service::network::error::{CreateNetworkError, DeleteNetworkError};
use sqlx::Connection;
use std::ops::Deref;
use tracing::error;

#[async_trait]
impl NetworkService for Daemon {
    async fn create_network(&self, ipv4: u16, ipv6: u16) -> Result<u32, CreateNetworkError> {
        let mut conn = self.conn.acquire().await?;
        let docker = self.docker.clone();
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
        .unwrap()
    }

    async fn delete_network(&self, network_id: u32) -> Result<(), DeleteNetworkError> {
        let Some(network) =
            Network::resolve::<DeleteNetworkError>(self.conn.deref(), network_id).await?
        else {
            return Err(DeleteNetworkError::NetworkNotFound);
        };
        let mut conn = self.conn.acquire().await?;
        let docker = self.docker.clone();
        tokio::task::spawn(async move {
            conn.transaction(move |tx| {
                Box::pin(async move { network.delete::<DeleteNetworkError>(&docker, tx).await })
            })
            .await?;
            Ok::<_, DeleteNetworkError>(())
        })
        .await
        .unwrap()?;

        Ok(())
    }
}
