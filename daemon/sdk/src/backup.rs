#[cfg(feature = "user")]
use crate::builder::user::DaemonSDKWithUserKey;
use crate::DaemonSDKError;
use crate::DefaultDaemonSDK;
use crate::*;
use daemon_common::payload::backup::BackupResponseEntry;
use http::Method;

// My ide doesn't like duplicate (this struct isn't actually used)
#[allow(unused)]
pub(crate) struct DaemonSDKStruct;

#[cfg_attr(not(feature = "user"), duplicate::duplicate_item[DaemonSDKStruct; [DefaultDaemonSDK]])]
#[cfg_attr(feature = "user", duplicate::duplicate_item[DaemonSDKStruct; [DefaultDaemonSDK]; [DaemonSDKWithUserKey]])]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
impl DaemonSDKStruct {
    pub async fn server_list_backups(
        &self,
        server: u32,
    ) -> Result<SdkVec<BackupResponseEntry>, DaemonSDKError> {
        let response = self
            .wrapper(
                Method::GET,
                format!("backup/{}", server),
                RequestBody::<()>::None,
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        #[allow(clippy::useless_conversion)]
        response
            .json::<Vec<BackupResponseEntry>>()
            .await
            .map_err(DaemonSDKError::Reqwest)
            .map(|vec| vec.into())
    }

    pub async fn server_create_backup(
        &self,
        server: u32,
        storage: String,
    ) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::PUT,
                format!("backup/{}/{}", server, storage),
                RequestBody::<()>::None,
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        #[allow(clippy::useless_conversion)]
        response
            .bytes()
            .await
            .map(|_| ())
            .map_err(DaemonSDKError::Reqwest)
    }

    pub async fn server_delete_backup(
        &self,
        server: u32,
        storage: String,
        backup: String,
    ) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::DELETE,
                format!("backup/{}/{}/{}", server, storage, backup),
                RequestBody::<()>::None,
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        #[allow(clippy::useless_conversion)]
        response
            .bytes()
            .await
            .map(|_| ())
            .map_err(DaemonSDKError::Reqwest)
    }

    pub async fn server_restore_backup(
        &self,
        server: u32,
        storage: String,
        backup: String,
    ) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::DELETE,
                format!("backup/{}/{}/{}/restore", server, storage, backup),
                RequestBody::<()>::None,
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        #[allow(clippy::useless_conversion)]
        response
            .bytes()
            .await
            .map(|_| ())
            .map_err(DaemonSDKError::Reqwest)
    }
}
