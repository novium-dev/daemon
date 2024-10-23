#[cfg(feature = "user")]
use crate::builder::user::DaemonSDKWithUserKey;
use crate::DefaultDaemonSDK;
use crate::{DaemonSDKError, RequestBody};
use daemon_common::payload::resource::ResourceResponse;
use http::Method;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

// My ide doesn't like duplicate (this struct isn't actually used)
#[allow(unused)]
pub(crate) struct DaemonSDKStruct;

#[cfg_attr(not(feature = "user"), duplicate::duplicate_item[DeamonSDKStruct; [DefaultDaemonSDK]])]
#[cfg_attr(feature = "user", duplicate::duplicate_item[DeamonSDKStruct; [DefaultDaemonSDK]; [DaemonSDKWithUserKey]])]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
impl DeamonSDKStruct {
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
