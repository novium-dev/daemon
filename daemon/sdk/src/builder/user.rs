use crate::builder::{DaemonSDKBuilder, DaemonSDKBuilderState};
use crate::{DaemonSDK, DaemonSDKExtra, DaemonSdkSealed};
use ed25519_dalek::SigningKey;
use std::sync::Arc;
use url::Url;

#[derive(Clone)]
pub struct WithUserKey(pub(crate) SigningKey);

impl DaemonSDKExtra for WithUserKey {}
impl DaemonSdkSealed for WithUserKey {}
impl DaemonSDKBuilderState for WithUserKey {
    type Inner = (String, String, Url, SigningKey);
}

impl DaemonSDKBuilder<()> {
    pub fn with_user_key(self, user_key: SigningKey) -> DaemonSDKBuilder<WithUserKey> {
        DaemonSDKBuilder((self.0.0, self.0.1, self.0.2, user_key))
    }
}

impl DaemonSDKBuilder<WithUserKey> {
    pub fn build(self) -> Arc<DaemonSDK<WithUserKey>> {
        Self::build0(self.0.0, self.0.1, self.0.2, WithUserKey(self.0.3))
    }
}
