pub use daemon_common as common;
use std::collections::HashMap;
pub mod server;

#[cfg(feature = "backups")]
pub mod backup;
pub mod builder;
#[cfg(feature = "database")]
pub mod database;
#[cfg(feature = "files")]
pub mod files;
pub mod resource;
#[cfg(feature = "user")]
pub mod user;

use http::{Method, StatusCode};
use reqwest::{Client, Response, Url};
use serde::Serialize;
use serde_json::Value;

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

pub(crate) enum RequestBody<T = ()>
where
    T: Serialize,
{
    None,
    Json(T),
    #[allow(dead_code)]
    Raw(Vec<u8>),
}

impl<T: DaemonSDKExtra> DaemonSDK<T> {
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
        }
        request.send().await.map_err(DaemonSDKError::Reqwest)
    }
}

#[derive(Debug)]
pub enum DaemonSDKError {
    Reqwest(reqwest::Error),
    URLParseError(url::ParseError),
    StatusCode(StatusCode),
    Limit(usize),
}
