use crate::{DaemonSDK, DaemonSDKError, DaemonSDKExtra, RequestBody};
use daemon_common::payload::network::NetworkPayload;
use daemon_common::payload::IdResponse;
use http::Method;

impl<T: DaemonSDKExtra> DaemonSDK<T> {
    pub async fn network_create(&self, ipv4: u16, ipv6: u16) -> Result<u32, DaemonSDKError> {
        let response = self
            .wrapper(
                Method::PUT,
                "network",
                RequestBody::Json(NetworkPayload { ipv4, ipv6 }),
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        response
            .json::<IdResponse>()
            .await
            .map(|response| response.id)
            .map_err(DaemonSDKError::Reqwest)
    }

    pub async fn network_delete(&self, network: u32) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::DELETE,
                format!("network/{}", network),
                RequestBody::None::<()>,
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        response
            .bytes()
            .await
            .map(|_| ())
            .map_err(DaemonSDKError::Reqwest)
    }
}
