use crate::DaemonSDKError;
use crate::*;
use bytes::Bytes;
use daemon_common::payload::files::{
    FileResponse, PathCompressionSourceQuery, PathDestinationQuery, PathDirectoryQuery, PathQuery,
    PathSymlinkQuery, PathUrlQuery, TwoPathQuery,
};
use daemon_common::payload::server::ServerFileWrite;
use futures_util::StreamExt;
use http::{HeaderMap, Method};
use reqwest::Response;
use std::pin::Pin;

impl<T: DaemonSDKExtra> DaemonSDK<T> {
    pub async fn server_list_files(
        &self,
        server: u32,
        path: String,
    ) -> Result<Vec<FileResponse>, DaemonSDKError> {
        let response = self
            .wrapper(
                Method::GET,
                format!("files/{}/list", server),
                RequestBody::<()>::None,
                Some(&PathQuery { path }),
            )
            .await?;
        if !response.status().is_success() {
            return Err(DaemonSDKError::StatusCode(response.status()));
        }
        response
            .json::<Vec<FileResponse>>()
            .await
            .map_err(DaemonSDKError::Reqwest)
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

    pub async fn server_download_file_to_stream(
        &self,
        server: u32,
        path: String,
        limit: Option<usize>,
    ) -> Result<
        (
            Pin<Box<dyn futures_core::Stream<Item = Result<Bytes, DaemonSDKError>> + Send + Sync>>,
            HeaderMap,
        ),
        DaemonSDKError,
    > {
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
        let headers = if let Some(length) = response.content_length() {
            if let Some(limit) = limit {
                if length > limit as u64 {
                    return Err(DaemonSDKError::Limit(limit));
                }
            }
            HeaderMap::from_iter([(http::header::CONTENT_LENGTH, length.into())])
        } else {
            HeaderMap::new()
        };
        Ok((
            match limit {
                None => Box::pin(
                    response
                        .bytes_stream()
                        .map(|item| item.map_err(DaemonSDKError::Reqwest)),
                ),
                Some(limit) => {
                    let stream = response.bytes_stream();

                    let mut count = 0u64;
                    Box::pin(stream.map(move |item| match item {
                        Ok(bytes) => {
                            count += bytes.len() as u64;
                            if (limit as u64) < count {
                                Err(DaemonSDKError::Limit(limit))
                            } else {
                                Ok(bytes)
                            }
                        }
                        Err(err) => Err(DaemonSDKError::Reqwest(err)),
                    }))
                }
            },
            headers,
        ))
    }

    pub async fn server_download_file_to_vec(
        &self,
        server: u32,
        path: String,
        limit: Option<usize>,
    ) -> Result<Vec<u8>, DaemonSDKError> {
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
            None => response
                .bytes()
                .await
                .map_err(DaemonSDKError::Reqwest)
                .map(|bytes| bytes.to_vec()),
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
                Ok(buf)
            }
        }
    }

    pub async fn server_write_file_by_vec(
        &self,
        server: u32,
        path: String,
        data: Vec<u8>,
    ) -> Result<(), DaemonSDKError> {
        let response = self
            .wrapper(
                Method::PUT,
                format!("files/{}/file", server),
                RequestBody::<()>::Raw(data),
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
    }

    pub async fn server_write_file_by_stream<S>(
        &self,
        server: u32,
        path: String,
        stream: S,
        length: Option<u64>,
    ) -> Result<(), DaemonSDKError>
    where
        S: futures_core::stream::TryStream + Send + 'static,
        S::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
        Bytes: From<S::Ok>,
    {
        let response = self
            .wrapper(
                Method::PUT,
                format!("files/{}/file", server),
                RequestBody::<()>::Body(Body::wrap_stream(stream), length),
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
    }

    pub async fn server_write_file_by_url(
        &self,
        server: u32,
        path: String,
        url: Url,
    ) -> Result<(), DaemonSDKError> {
        let response: Response = self
            .wrapper(
                Method::GET,
                format!("files/{}/url", server),
                RequestBody::<()>::None,
                Some(&PathUrlQuery {
                    path,
                    url: url.to_string(),
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

    pub async fn server_write_file_by_template(
        &self,
        server: u32,
        path: String,
        template: ServerFileWrite,
    ) -> Result<(), DaemonSDKError> {
        let response: Response = self
            .wrapper(
                Method::POST,
                format!("files/{}/template", server),
                RequestBody::Json(template),
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
    }

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

    pub async fn server_mkdir(
        &self,
        server: u32,
        path: String,
        recursive: bool,
    ) -> Result<(), DaemonSDKError> {
        let response: Response = self
            .wrapper(
                Method::GET,
                format!("files/{}/mkdir", server),
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

    pub async fn server_create_archive(
        &self,
        server: u32,
        path: String,
        src: String,
        compression: String,
    ) -> Result<(), DaemonSDKError> {
        let response: Response = self
            .wrapper(
                Method::GET,
                format!("files/{}/archive/create", server),
                RequestBody::<()>::None,
                Some(&PathCompressionSourceQuery {
                    path,
                    src,
                    compression,
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

    pub async fn server_unpack_archive(
        &self,
        server: u32,
        path: String,
        dest: String,
    ) -> Result<(), DaemonSDKError> {
        let response: Response = self
            .wrapper(
                Method::GET,
                format!("files/{}/archive/unpack", server),
                RequestBody::<()>::None,
                Some(&PathDestinationQuery { path, dest }),
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
