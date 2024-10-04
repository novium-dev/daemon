#[cfg(feature = "websocket")]
pub mod websocket;

use reqwest::{IntoUrl, Url};
pub use daemon_common as common;

pub(crate) trait DaemonSDKExtra {}
impl DaemonSDKExtra for () {}

trait DaemonSDKBuilderState {}
pub enum RequiresIdentifier {}
pub enum RequiresToken {}
pub enum RequiresBaseURL {}
impl DaemonSDKBuilderState for RequiresIdentifier {}
impl DaemonSDKBuilderState for RequiresToken {}
impl DaemonSDKBuilderState for RequiresBaseURL {}
impl DaemonSDKBuilderState for () {}

pub struct DaemonSDK<T: DaemonSDKExtra> {
    identifier: String,
    token: String,
    base_url: Url,
    extra: T,
}

pub struct DaemonSDKBuilder<T: DaemonSDKBuilderState> {
    identifier: Option<String>,
    token: Option<String>,
    base_url: Option<Url>,
    extra: Option<T>,
}

impl<T: DaemonSDKExtra> DaemonSDK<T> {

    pub fn builder() -> DaemonSDKBuilder<RequiresIdentifier> {
        DaemonSDKBuilder {
            identifier: None,
            token: None,
            base_url: None,
            extra: None,
        }
    }

}

impl DaemonSDKBuilder<RequiresIdentifier> {

    pub fn with_identifier(self, identifier: String) -> DaemonSDKBuilder<RequiresToken> {
        DaemonSDKBuilder {
            identifier: Some(identifier),
            token: None,
            base_url: None,
            extra: None,
        }
    }

}

impl DaemonSDKBuilder<RequiresToken> {

    pub fn with_token(self, token: String) -> DaemonSDKBuilder<RequiresBaseURL> {
        DaemonSDKBuilder {
            identifier: self.identifier,
            token: Some(token),
            base_url: None,
            extra: None,
        }
    }

}

impl DaemonSDKBuilder<RequiresBaseURL> {

    pub fn with_url(self, base_url: impl IntoUrl) -> Result<DaemonSDKBuilder<()>, reqwest::Error> {
        Ok(DaemonSDKBuilder {
            identifier: self.identifier,
            token: self.token,
            base_url: Some(base_url.into_url()?),
            extra: Some(()),
        })
    }

}

impl<T: DaemonSDKExtra + DaemonSDKBuilderState> DaemonSDKBuilder<T> {
    pub fn build(self) -> DaemonSDK<T> {
        DaemonSDK {
            identifier: self.identifier.unwrap(),
            token: self.token.unwrap(),
            base_url: self.base_url.unwrap(),
            extra: self.extra.unwrap()
        }
    }
}

