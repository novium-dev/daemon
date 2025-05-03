use crate::sdk::{DaemonSDK, DaemonSDKExtra, DaemonSdkSealed};
use ed25519_dalek::SigningKey;
use http::{HeaderMap, HeaderValue};
use reqwest::{Client, IntoUrl, Url};

#[allow(private_bounds)]
pub trait DaemonSDKBuilderState: DaemonSdkSealed {
    type Inner;
}
pub enum RequiresIdentifier {}
pub enum RequiresToken {}
pub enum RequiresBaseURL {}

#[derive(Clone)]
pub struct WithUserKey(pub(crate) SigningKey);

impl DaemonSDKExtra for WithUserKey {}
impl DaemonSdkSealed for WithUserKey {}

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

pub struct DaemonSDKBuilder<T: DaemonSDKBuilderState + Send + Sync>(pub(crate) T::Inner);

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
    pub fn build(self) -> DaemonSDK<()> {
        Self::build0(self.0.0, self.0.1, self.0.2, ())
    }
}

impl<T: DaemonSDKExtra + DaemonSDKBuilderState + Send + Sync> DaemonSDKBuilder<T> {
    pub(crate) fn build0(
        identifier: String,
        token: String,
        base_url: Url,
        extra: T,
    ) -> DaemonSDK<T> {
        DaemonSDK {
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
        }
    }
}

impl DaemonSDKBuilderState for WithUserKey {
    type Inner = (String, String, Url, SigningKey);
}

impl DaemonSDKBuilder<()> {
    pub fn with_user_key(self, user_key: SigningKey) -> DaemonSDKBuilder<WithUserKey> {
        DaemonSDKBuilder((self.0.0, self.0.1, self.0.2, user_key))
    }
}

impl DaemonSDKBuilder<WithUserKey> {
    pub fn build(self) -> DaemonSDK<WithUserKey> {
        Self::build0(self.0.0, self.0.1, self.0.2, WithUserKey(self.0.3))
    }
}
