#[cfg(feature = "user")]
use crate::builder::user::DaemonSDKWithUserKey;
use crate::DaemonSDKError;
use crate::DefaultDaemonSDK;
use crate::*;
use daemon_common::payload::files::{
    FileResponse, PathDirectoryQuery, PathQuery, PathSymlinkQuery, TwoPathQuery,
};
use futures_util::StreamExt;
use http::Method;
use reqwest::Response;

// My ide doesn't like duplicate (this struct isn't actually used)
#[allow(unused)]
pub(crate) struct DaemonSDKStruct;

#[cfg_attr(not(feature = "user"), duplicate::duplicate_item[DaemonSDKStruct; [DefaultDaemonSDK]])]
#[cfg_attr(feature = "user", duplicate::duplicate_item[DaemonSDKStruct; [DefaultDaemonSDK]; [DaemonSDKWithUserKey]])]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
impl DaemonSDKStruct {
    pub async fn server_list_files(
        &self,
        server: u32,
        path: String,
    ) -> Result<SdkVec<FileResponse>, DaemonSDKError> {
        let response = self
            .wrapper(
                Method::GET,
                format!("files/{}/ls", server),
                RequestBody::<()>::None,
                Some(&PathQuery { path }),
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        #[allow(clippy::useless_conversion)]
        response
            .json::<Vec<FileResponse>>()
            .await
            .map_err(DaemonSDKError::Reqwest)
            .map(|vec| vec.into())
    }

    pub async fn server_stat_file(
        &self,
        server: u32,
        path: String,
    ) -> Result<FileResponse, DaemonSDKError> {
        let response = self
            .wrapper(
                Method::GET,
                format!("files/{}/stat", server),
                RequestBody::<()>::None,
                Some(&PathQuery { path }),
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        response
            .json::<FileResponse>()
            .await
            .map_err(DaemonSDKError::Reqwest)
    }

    pub async fn server_rename_file(
        &self,
        server: u32,
        from: String,
        to: String,
    ) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::GET,
                format!("files/{}/rename", server),
                RequestBody::<()>::None,
                Some(&TwoPathQuery { from, to }),
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

    pub async fn server_copy_file(
        &self,
        server: u32,
        from: String,
        to: String,
    ) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::GET,
                format!("files/{}/copy", server),
                RequestBody::<()>::None,
                Some(&TwoPathQuery { from, to }),
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

    pub async fn server_download_file(
        &self,
        server: u32,
        path: String,
        limit: Option<usize>,
    ) -> Result<SdkVec<u8>, DaemonSDKError> {
        let response: Response = self
            .wrapper(
                Method::GET,
                format!("files/{}/file", server),
                RequestBody::<()>::None,
                Some(&PathQuery { path }),
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        match limit {
            None =>
            {
                #[allow(clippy::useless_conversion)]
                response
                    .bytes()
                    .await
                    .map_err(DaemonSDKError::Reqwest)
                    .map(|bytes| bytes.to_vec().into())
            }
            Some(limit) => {
                let mut buf = Vec::new();
                let mut stream = response.bytes_stream();
                while let Some(entry) = stream.next().await {
                    let entry = entry.map_err(DaemonSDKError::Reqwest)?;
                    if limit < (buf.len() + entry.len()) {
                        return Err(DaemonSDKError::Limit(limit));
                    }
                    buf.extend_from_slice(&entry);
                }
                Ok(
                    #[allow(clippy::useless_conversion)]
                    buf.into(),
                )
            }
        }
    }

    /* TODO: wasm-bindgen doesn't like SdkVec for some reason, I will fix it later
    pub async fn server_write_file(
        &self,
        server: u32,
        path: String,
        data: SdkVec<u8>,
    ) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::PUT,
                format!("files/{}/file", server),
                RequestBody::<()>::Raw(
                    #[allow(clippy::useless_conversion)]
                    data.into(),
                ),
                Some(&PathQuery { path }),
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
    }*/

    pub async fn server_delete_file(
        &self,
        server: u32,
        path: String,
        recursive: bool,
    ) -> Result<(), DaemonSDKError> {
        let response: Response = self
            .wrapper(
                Method::GET,
                format!("files/{}/delete", server),
                RequestBody::<()>::None,
                Some(&PathDirectoryQuery { path, recursive }),
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

    pub async fn server_mkdir(&self, server: u32, path: String) -> Result<(), DaemonSDKError> {
        let response: Response = self
            .wrapper(
                Method::GET,
                format!("files/{}/mkdir", server),
                RequestBody::<()>::None,
                Some(&PathDirectoryQuery {
                    path,
                    recursive: false,
                }),
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

    pub async fn server_symlink(
        &self,
        server: u32,
        path: String,
        content: String,
    ) -> Result<(), DaemonSDKError> {
        let response: Response = self
            .wrapper(
                Method::GET,
                format!("files/{}/symlink", server),
                RequestBody::<()>::None,
                Some(&PathSymlinkQuery { path, content }),
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
