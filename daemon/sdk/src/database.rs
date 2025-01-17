use crate::DaemonSDKError;
use crate::*;
use daemon_common::payload::database::{
    DatabaseCreatePayload, DatabaseModifyPayload, DatabasePasswordlessPayload, DatabaseUsersPayload,
};
use daemon_common::payload::IdResponse;
use http::Method;
impl<T: DaemonSDKExtra> DaemonSDK<T> {
    pub async fn create_database(&self, name_suffix: String) -> Result<u32, DaemonSDKError> {
        let response = self
            .wrapper(
                Method::PUT,
                "database",
                RequestBody::Json(DatabaseCreatePayload { name_suffix }),
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
                RequestBody::Json(DatabaseModifyPayload {
                    users: users.0,
                    passwordless: passwordless.0,
                }),
                None::<&()>,
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
                RequestBody::<()>::None,
                None::<&()>,
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        Ok(())
    }
}
