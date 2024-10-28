use crate::DaemonSDKError;
use crate::*;
use daemon_common::payload::backup::BackupResponseEntry;
use http::Method;

impl<T: DaemonSDKExtra> DaemonSDK<T> {
    pub async fn server_list_backups(
        &self,
        server: u32,
    ) -> Result<Vec<BackupResponseEntry>, DaemonSDKError> {
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
        response
            .json::<Vec<BackupResponseEntry>>()
            .await
            .map_err(DaemonSDKError::Reqwest)
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
        response
            .bytes()
            .await
            .map(|_| ())
            .map_err(DaemonSDKError::Reqwest)
    }
}
