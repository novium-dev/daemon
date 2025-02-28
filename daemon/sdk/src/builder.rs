#[cfg(feature = "user")]
pub mod user;

use crate::{DaemonSDK, DaemonSDKExtra, DaemonSdkSealed};
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
    type Inner = ();
}
impl DaemonSDKBuilderState for RequiresToken {
    type Inner = String;
}
impl DaemonSDKBuilderState for RequiresBaseURL {
    type Inner = (String, String);
}
impl DaemonSDKBuilderState for () {
    type Inner = (String, String, Url);
}
impl DaemonSdkSealed for RequiresIdentifier {}
impl DaemonSdkSealed for RequiresToken {}
impl DaemonSdkSealed for RequiresBaseURL {}

pub struct DaemonSDKBuilder<T: DaemonSDKBuilderState>(pub(crate) T::Inner);

impl DaemonSDK<()> {
    pub fn builder() -> DaemonSDKBuilder<RequiresIdentifier> {
        DaemonSDKBuilder(())
    }
}

impl DaemonSDKBuilder<RequiresIdentifier> {
    pub fn with_identifier(self, identifier: String) -> DaemonSDKBuilder<RequiresToken> {
        DaemonSDKBuilder(identifier)
    }
}

impl DaemonSDKBuilder<RequiresToken> {
    pub fn with_token(self, token: String) -> DaemonSDKBuilder<RequiresBaseURL> {
        DaemonSDKBuilder((self.0, token))
    }
}

impl DaemonSDKBuilder<RequiresBaseURL> {
    pub fn with_url(self, base_url: impl IntoUrl) -> Result<DaemonSDKBuilder<()>, reqwest::Error> {
        Ok(DaemonSDKBuilder((self.0.0, self.0.1, base_url.into_url()?)))
    }
}

impl DaemonSDKBuilder<()> {
    pub fn build(self) -> Arc<DaemonSDK<()>> {
        Self::build0(self.0.0, self.0.1, self.0.2, ())
    }
}

impl<T: DaemonSDKExtra + DaemonSDKBuilderState> DaemonSDKBuilder<T> {
    pub(crate) fn build0(
        identifier: String,
        token: String,
        base_url: Url,
        extra: T,
    ) -> Arc<DaemonSDK<T>> {
        Arc::new(DaemonSDK {
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
