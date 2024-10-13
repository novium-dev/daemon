#[cfg(feature = "websocket")]
use crate::builder::user::DaemonSDKWithUserKey;
use crate::DaemonSDKError;
use crate::DefaultDaemonSDK;
#[cfg(target_arch = "wasm32")]
use crate::*;
use daemon_common::payload::database::{
    DatabasePasswordlessPayload, DatabasePayload, DatabaseUsersPayload,
};
use daemon_common::payload::IdResponse;
use http::Method;

// My ide doesn't like duplicate (this struct isn't actually used)
#[allow(unused)]
pub(crate) struct DaemonSDKStruct;

#[cfg_attr(not(feature = "websocket"), duplicate::duplicate_item[DaemonSDKStruct; [DefaultDaemonSDK]])]
#[cfg_attr(feature = "websocket", duplicate::duplicate_item[DaemonSDKStruct; [DefaultDaemonSDK]; [DaemonSDKWithUserKey]])]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
impl DaemonSDKStruct {
    pub async fn create_database(
        &self,
        name_suffix: String,
        users: DatabaseUsersPayload,
        passwordless: DatabasePasswordlessPayload,
    ) -> Result<u32, DaemonSDKError> {
        let response = self
            .wrapper(
                Method::PUT,
                "database",
                Some(&DatabasePayload {
                    name_suffix: Some(name_suffix),
                    users: users.0,
                    passwordless: passwordless.0,
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

    pub async fn update_database(
        &self,
        database: u32,
        users: DatabaseUsersPayload,
        passwordless: DatabasePasswordlessPayload,
    ) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::PATCH,
                format!("database/{}", database),
                Some(&DatabasePayload {
                    name_suffix: None,
                    users: users.0,
                    passwordless: passwordless.0,
                }),
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }

    pub async fn delete_database(&self, database: u32) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::DELETE,
                format!("database/{}", database),
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }
}
