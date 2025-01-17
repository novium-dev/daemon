use crate::{DaemonSDK, DaemonSDKError, DaemonSDKExtra, RequestBody};
use daemon_common::payload::server::{
    ServerBuildPayload, ServerNetworkPayload, ServerPayload, ServerPortEntryPayload,
    ServerResourcePayload,
};
use daemon_common::payload::IdResponse;
use daemon_common::server::PublicServerState;
use http::Method;

impl<T: DaemonSDKExtra> DaemonSDK<T> {
    pub async fn get_server(&self, server: u32) -> Result<PublicServerState, DaemonSDKError> {
        let response = self
            .wrapper(
                Method::GET,
                format!("server/{}", server),
                RequestBody::<()>::None,
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        response.json().await.map_err(DaemonSDKError::Reqwest)
    }

    pub async fn create_server(
        &self,
        storage: String,
        resource: ServerResourcePayload,
        build: ServerBuildPayload,
        ports: Vec<ServerPortEntryPayload>,
        network: ServerNetworkPayload,
    ) -> Result<u32, DaemonSDKError> {
        let response = self
            .wrapper(
                Method::PUT,
                "server",
                RequestBody::Json(ServerPayload {
                    storage: Some(storage),
                    resource: Some(resource),
                    build: Some(build),
                    ports: Some(ports),
                    network: Some(network),
                }),
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        response
            .json::<IdResponse>()
            .await
            .map_err(DaemonSDKError::Reqwest)
            .map(|response| response.id)
    }

    pub async fn update_server(
        &self,
        server: u32,
        resource: Option<ServerResourcePayload>,
        build: Option<ServerBuildPayload>,
        ports: Option<Vec<ServerPortEntryPayload>>,
        network: Option<ServerNetworkPayload>,
    ) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::PATCH,
                format!("server/{}", server),
                RequestBody::Json(ServerPayload {
                    storage: None,
                    resource,
                    build,
                    ports,
                    network,
                }),
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }

    pub async fn delete_server(&self, server: u32) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::DELETE,
                format!("server/{}", server),
                RequestBody::<()>::None,
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }

    pub async fn start_server(&self, server: u32) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::GET,
                format!("server/{}/start", server),
                RequestBody::<()>::None,
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }

    pub async fn restart_server(&self, server: u32) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::GET,
                format!("server/{}/restart", server),
                RequestBody::<()>::None,
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }

    pub async fn stop_server(&self, server: u32) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::GET,
                format!("server/{}/stop", server),
                RequestBody::<()>::None,
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }

    pub async fn kill_server(&self, server: u32) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::GET,
                format!("server/{}/kill", server),
                RequestBody::<()>::None,
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }

    pub async fn exec_server(
        &self,
        server: u32,
        command: Vec<String>,
    ) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::POST,
                format!("server/{}/exec", server),
                RequestBody::Json(command),
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }

    pub async fn server_status(&self, server: u32) -> Result<PublicServerState, DaemonSDKError> {
        let response = self
            .wrapper(
                Method::GET,
                format!("server/{}", server),
                RequestBody::None::<()>,
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        response
            .json::<PublicServerState>()
            .await
            .map_err(DaemonSDKError::Reqwest)
    }
}
