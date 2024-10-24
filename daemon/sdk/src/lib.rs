pub use daemon_common as common;
use std::collections::HashMap;
use std::ops::Deref;
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

impl Deref for DaemonSDK<()> {
    type Target = DefaultDaemonSDK;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
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

impl<T: DaemonSDKExtra> InternalDaemonSDK<T> {
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
                url.query_pairs_mut().append_pair(&key, &val.to_string());
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
    #[cfg(feature = "user")]
    InvalidWebsocketKey,
    Limit(usize),
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
        Limit { limit: usize },
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
            #[cfg(feature = "user")]
            DaemonSDKError::InvalidWebsocketKey => wasm::DaemonSDKError::Other {
                message: "Websocket Key is invalid! (not exactly 32 bytes)".to_owned(),
            },
            DaemonSDKError::Limit(limit) => wasm::DaemonSDKError::Limit { limit },
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

#[cfg(not(target_arch = "wasm32"))]
pub type SdkVec<T> = Vec<T>;

#[cfg(target_arch = "wasm32")]
pub struct SdkVec<T>(Vec<T>);

#[cfg(target_arch = "wasm32")]
impl<T> Into<Vec<T>> for SdkVec<T> {
    fn into(self) -> Vec<T> {
        self.0
    }
}

#[cfg(target_arch = "wasm32")]
impl<T: Into<JsValue>> From<Vec<T>> for SdkVec<T> {
    fn from(value: Vec<T>) -> Self {
        Self(value)
    }
}

#[cfg(target_arch = "wasm32")]
impl<T> Deref for SdkVec<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(target_arch = "wasm32")]
impl<T> std::ops::DerefMut for SdkVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(target_arch = "wasm32")]
impl<T: Into<JsValue>> Into<JsValue> for SdkVec<T> {
    fn into(self) -> JsValue {
        let array = js_sys::Array::new();
        for v in self.0 {
            array.push(&v.into());
        }
        array.into()
    }
}

#[cfg(target_arch = "wasm32")]
impl<T: wasm_bindgen::describe::WasmDescribe> wasm_bindgen::describe::WasmDescribe for SdkVec<T> {
    fn describe() {
        wasm_bindgen::describe::inform(wasm_bindgen::describe::VECTOR);
        T::describe();
    }
}

#[cfg(target_arch = "wasm32")]
impl<T: wasm_bindgen::convert::FromWasmAbi<Abi = u32>> wasm_bindgen::convert::FromWasmAbi
    for SdkVec<T>
{
    type Abi = u32;

    unsafe fn from_abi(js: Self::Abi) -> Self {
        use wasm_bindgen::convert::IntoWasmAbi;
        let mut vec = Vec::new();
        let array = js_sys::Array::from_abi(js);
        for entry in array {
            vec.push(T::from_abi(entry.into_abi()));
        }
        SdkVec(vec)
    }
}
