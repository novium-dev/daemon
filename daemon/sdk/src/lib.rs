#![forbid(unsafe_code)]
#![warn(clippy::string_to_string)]

pub use daemon_common as common;
use std::collections::HashMap;
use ed25519_dalek::SigningKey;

pub mod builder;
#[cfg(feature = "database")]
pub mod database;
pub mod network;
pub mod resource;
pub mod schedule;
#[cfg(feature = "user")]
pub mod user;

use daemon_common::FrameworkClientImpl;
use daemon_common::service::DaemonServicesClientImpl;
use daemon_common::service::backup::BackupServiceClientImpl;
use daemon_common::service::server::ServerServiceClientImpl;
use http::{Method, StatusCode};
use reqwest::{Body, Client, Response, Url};
use serde::Serialize;
use serde_json::Value;
use daemon_common::service::files::FilesServiceClientImpl;
use daemon_common::user::UserKeyProvider;
use crate::builder::user::WithUserKey;

#[allow(private_bounds)]
pub trait DaemonSDKExtra: DaemonSdkSealed + Sized {}
pub(crate) trait DaemonSdkSealed {}
impl DaemonSDKExtra for () {}
impl DaemonSdkSealed for () {}

#[derive(Debug, Clone)]
pub struct DaemonSDK<T: DaemonSDKExtra> {
    client: Client,
    identifier: String,
    base_url: Url,
    #[allow(unused)]
    extra: T,
}

#[deprecated]
pub(crate) enum RequestBody<T = ()>
where
    T: Serialize,
{
    None,
    Json(T),
    #[allow(dead_code)]
    Raw(Vec<u8>),
    #[allow(dead_code)]
    Body(Body, Option<u64>),
}

impl<T: DaemonSDKExtra> DaemonSDK<T> {
    #[deprecated]
    pub(crate) async fn wrapper<J: Serialize>(
        &self,
        method: Method,
        path: impl Into<String>,
        body: RequestBody<J>,
        query: Option<&impl Serialize>,
    ) -> Result<Response, DaemonSDKError> {
        let path = path.into();
        let mut url = self.base_url.join(&path).unwrap();
        if let Some(query) = query {
            let query: HashMap<String, Value> =
                serde_json::from_value(serde_json::to_value(query).unwrap()).unwrap();
            for (key, val) in query {
                url.query_pairs_mut().append_pair(
                    &key,
                    &match val {
                        Value::Null => "null".to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Number(n) => n.to_string(),
                        Value::String(s) => s,
                        _ => unreachable!(),
                    },
                );
            }
        }
        let mut request = self.client.request(method, url);
        match body {
            RequestBody::None => {}
            RequestBody::Json(body) => {
                request = request.json(&body);
            }
            RequestBody::Raw(bytes) => {
                request = request.body(bytes);
            }
            RequestBody::Body(body, length) => {
                request = request.body(body);
                if let Some(length) = length {
                    request = request.header(http::header::CONTENT_LENGTH, length);
                }
            }
        }
        request.send().await.map_err(DaemonSDKError::Reqwest)
    }
}

#[deprecated]
#[derive(Debug)]
pub enum DaemonSDKError {
    Reqwest(reqwest::Error),
    URLParseError(url::ParseError),
    StatusCode(StatusCode),
    Limit(usize),
}

impl<T: DaemonSDKExtra> ServerServiceClientImpl for DaemonSDK<T> {}

impl<T: DaemonSDKExtra> FrameworkClientImpl for DaemonSDK<T> {
    fn client(&self) -> &Client {
        &self.client
    }

    fn base_url(&self) -> &Url {
        &self.base_url
    }
}

impl<T: DaemonSDKExtra> DaemonServicesClientImpl for DaemonSDK<T> {}
impl<T: DaemonSDKExtra> BackupServiceClientImpl for DaemonSDK<T> {}
impl<T: DaemonSDKExtra> FilesServiceClientImpl for DaemonSDK<T> {}

impl UserKeyProvider for DaemonSDK<WithUserKey> {
    fn node_id(&self) -> &str {
        &self.identifier
    }

    fn key(&self) -> &SigningKey {
        &self.extra.0
    }
}
