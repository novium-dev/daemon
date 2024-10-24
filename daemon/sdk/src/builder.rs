#[cfg(target_arch = "wasm32")]
use crate::*;
#[cfg(feature = "user")]
pub mod user;
#[cfg(target_arch = "wasm32")]
pub mod wasm;

use crate::{DaemonSDK, DaemonSDKExtra, DaemonSdkSealed, DefaultDaemonSDK, InternalDaemonSDK};
use http::{HeaderMap, HeaderValue};
use reqwest::{Client, IntoUrl, Url};
use std::sync::Arc;

#[allow(private_bounds)]
pub trait DaemonSDKBuilderState: DaemonSdkSealed {
    type Inner;
}
pub enum RequiresIdentifier {}
pub enum RequiresToken {}
pub enum RequiresBaseURL {}
impl DaemonSDKBuilderState for RequiresIdentifier {
    type Inner = DaemonSDKBuilderRequiresIdentifier;
}
impl DaemonSDKBuilderState for RequiresToken {
    type Inner = DaemonSDKBuilderRequiresToken;
}
impl DaemonSDKBuilderState for RequiresBaseURL {
    type Inner = DaemonSDKBuilderRequiresBaseURL;
}
impl DaemonSDKBuilderState for () {
    type Inner = DaemonSDKBuilderBuildable;
}
impl DaemonSdkSealed for RequiresIdentifier {}
impl DaemonSdkSealed for RequiresToken {}
impl DaemonSdkSealed for RequiresBaseURL {}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub struct DaemonSDKBuilderRequiresIdentifier {}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub struct DaemonSDKBuilderRequiresToken {
    identifier: String,
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub struct DaemonSDKBuilderRequiresBaseURL {
    identifier: String,
    token: String,
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub struct DaemonSDKBuilderBuildable {
    pub(crate) identifier: String,
    pub(crate) token: String,
    pub(crate) base_url: Url,
}

pub struct DaemonSDKBuilder<T: DaemonSDKBuilderState>(pub(crate) T::Inner);

impl DaemonSDK<()> {
    pub fn builder() -> DaemonSDKBuilder<RequiresIdentifier> {
        DaemonSDKBuilder(DaemonSDKBuilderRequiresIdentifier {})
    }
}

impl DaemonSDKBuilder<RequiresIdentifier> {
    pub fn with_identifier(self, identifier: String) -> DaemonSDKBuilder<RequiresToken> {
        DaemonSDKBuilder(DaemonSDKBuilderRequiresToken { identifier })
    }
}

impl DaemonSDKBuilder<RequiresToken> {
    pub fn with_token(self, token: String) -> DaemonSDKBuilder<RequiresBaseURL> {
        DaemonSDKBuilder(DaemonSDKBuilderRequiresBaseURL {
            identifier: self.0.identifier,
            token,
        })
    }
}

impl DaemonSDKBuilder<RequiresBaseURL> {
    pub fn with_url(self, base_url: impl IntoUrl) -> Result<DaemonSDKBuilder<()>, reqwest::Error> {
        Ok(DaemonSDKBuilder(DaemonSDKBuilderBuildable {
            identifier: self.0.identifier,
            token: self.0.token,
            base_url: base_url.into_url()?,
        }))
    }
}

impl DaemonSDKBuilder<()> {
    pub fn build(self) -> DaemonSDK<()> {
        DaemonSDK(DefaultDaemonSDK(Self::build0(
            self.0.identifier,
            self.0.token,
            self.0.base_url,
            (),
        )))
    }
}

impl<T: DaemonSDKExtra + DaemonSDKBuilderState> DaemonSDKBuilder<T> {
    pub(crate) fn build0(
        identifier: String,
        token: String,
        base_url: Url,
        extra: T,
    ) -> Arc<InternalDaemonSDK<T>> {
        Arc::new(InternalDaemonSDK {
            client: Client::builder()
                .default_headers(HeaderMap::from_iter([(
                    http::header::AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
                )]))
                .build()
                .unwrap(),
            identifier,
            base_url,
            extra,
        })
    }
}
