#[cfg(feature = "websocket")]
use crate::builder::websocket::DaemonSDKWithWebsocketKey;
use crate::DaemonSDKError;
use crate::DefaultDaemonSDK;
use daemon_common::payload::server::{
    ServerBuildPayload, ServerPayload, ServerPortsPayload, ServerResourcePayload,
};
use daemon_common::payload::IdResponse;
use daemon_common::server::PublicServerState;
use http::Method;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

// My ide doesn't like duplicate (this struct isn't actually used)
#[allow(unused)]
pub(crate) struct DaemonSDKStruct;

#[cfg_attr(not(feature = "websocket"), duplicate::duplicate_item[DeamonSDKStruct; [DefaultDaemonSDK]])]
#[cfg_attr(feature = "websocket", duplicate::duplicate_item[DeamonSDKStruct; [DefaultDaemonSDK]; [DaemonSDKWithWebsocketKey]])]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
impl DeamonSDKStruct {
    pub async fn get_server(&self, server: u32) -> Result<PublicServerState, DaemonSDKError> {
        let response = self
            .wrapper(Method::GET, format!("server/{}", server), None::<&()>)
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        response.json().await.map_err(DaemonSDKError::Reqwest)
    }

    pub async fn create_server(
        &self,
        resource: ServerResourcePayload,
        build: ServerBuildPayload,
        ports: ServerPortsPayload,
    ) -> Result<u32, DaemonSDKError> {
        let response = self
            .wrapper(
                Method::PUT,
                "server",
                Some(&ServerPayload {
                    resource: Some(resource),
                    build: Some(build),
                    ports: Some(ports.0),
                }),
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
        ports: Option<ServerPortsPayload>,
    ) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::PATCH,
                format!("server/{}", server),
                Some(&ServerPayload {
                    resource,
                    build,
                    ports: ports.map(|payload| payload.0),
                }),
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }

    pub async fn delete_server(&self, server: u32) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(Method::DELETE, format!("server/{}", server), None::<&()>)
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }

    pub async fn start_server(&self, server: u32) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(Method::GET, format!("server/{}/start", server), None::<&()>)
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
            .wrapper(Method::GET, format!("server/{}/stop", server), None::<&()>)
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }

    pub async fn kill_server(&self, server: u32) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(Method::GET, format!("server/{}/kill", server), None::<&()>)
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }
}
