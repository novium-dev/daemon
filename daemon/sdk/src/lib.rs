pub use daemon_common as common;
use std::ops::Deref;
pub mod server;

pub mod builder;
#[cfg(feature = "database")]
pub mod database;
pub mod resource;
#[cfg(feature = "user")]
pub mod user;

use http::{Method, StatusCode};
use reqwest::{Client, Response, Url};
use serde::Serialize;
use std::sync::Arc;
#[cfg(target_arch = "wasm32")]
use tsify::Tsify;
#[cfg(target_arch = "wasm32")]
pub use wasm_bindgen::prelude::*;

#[allow(private_bounds)]
pub trait DaemonSDKExtra: DaemonSdkSealed + Sized {
    type Inner: Deref<Target = InternalDaemonSDK<Self>> + Clone;
}
pub(crate) trait DaemonSdkSealed {}
impl DaemonSDKExtra for () {
    type Inner = DefaultDaemonSDK;
}
impl DaemonSdkSealed for () {}

#[derive(Clone)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub struct DefaultDaemonSDK(pub(crate) Arc<InternalDaemonSDK<()>>);

impl Deref for DefaultDaemonSDK {
    type Target = InternalDaemonSDK<()>;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

pub struct InternalDaemonSDK<T: DaemonSDKExtra> {
    client: Client,
    identifier: String,
    base_url: Url,
    #[allow(unused)]
    extra: T,
}

#[derive(Clone)]
pub struct DaemonSDK<T: DaemonSDKExtra>(T::Inner);

impl<T: DaemonSDKExtra> Deref for DaemonSDK<T> {
    type Target = InternalDaemonSDK<T>;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl<T: DaemonSDKExtra> InternalDaemonSDK<T> {
    pub(crate) async fn wrapper(
        &self,
        method: Method,
        path: impl Into<String>,
        body: Option<&impl Serialize>,
    ) -> Result<Response, DaemonSDKError> {
        let path = path.into();
        let mut request = self
            .client
            .request(method, self.base_url.join(&path).unwrap());
        if let Some(body) = body {
            request = request.json(body);
        }
        request.send().await.map_err(DaemonSDKError::Reqwest)
    }
}

#[derive(Debug)]
pub enum DaemonSDKError {
    Reqwest(reqwest::Error),
    URLParseError(url::ParseError),
    StatusCode(StatusCode),
    #[cfg(feature = "websocket")]
    InvalidWebsocketKey,
}

#[cfg(target_arch = "wasm32")]
pub mod wasm {
    use daemon_common::payload::Tsify;
    use serde::Serialize;

    #[derive(Tsify, Serialize)]
    #[serde(untagged)]
    #[tsify(into_wasm_abi)]
    pub enum DaemonSDKError {
        Other { message: String },
        StatusCode { message: String, code: u16 },
    }
}

#[cfg(target_arch = "wasm32")]
impl Into<wasm::DaemonSDKError> for DaemonSDKError {
    fn into(self) -> wasm::DaemonSDKError {
        match self {
            DaemonSDKError::Reqwest(error) => wasm::DaemonSDKError::Other {
                message: error.to_string(),
            },
            DaemonSDKError::URLParseError(error) => wasm::DaemonSDKError::Other {
                message: error.to_string(),
            },
            DaemonSDKError::StatusCode(code) => wasm::DaemonSDKError::StatusCode {
                code: code.as_u16(),
                message: code
                    .canonical_reason()
                    .map(String::from)
                    .unwrap_or_default(),
            },
            #[cfg(feature = "websocket")]
            DaemonSDKError::InvalidWebsocketKey => wasm::DaemonSDKError::Other {
                message: "Websocket Key is invalid! (not exactly 32 bytes)".to_owned(),
            },
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Into<JsValue> for DaemonSDKError {
    fn into(self) -> JsValue {
        let err: wasm::DaemonSDKError = self.into();
        err.into_js().unwrap().into()
    }
}
