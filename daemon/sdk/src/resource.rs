use crate::{DaemonSDK, DaemonSDKError, DaemonSDKExtra, RequestBody};
use daemon_common::payload::resource::ResourceResponse;
use http::Method;

impl<T: DaemonSDKExtra> DaemonSDK<T> {
    pub async fn get_resource(&self) -> Result<ResourceResponse, DaemonSDKError> {
        let response = self
            .wrapper(
                Method::GET,
                "resource",
                RequestBody::<()>::None,
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        response.json().await.map_err(DaemonSDKError::Reqwest)
    }
}
