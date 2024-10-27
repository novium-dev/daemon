use crate::builder::{DaemonSDKBuilder, DaemonSDKBuilderState};
#[cfg(target_arch = "wasm32")]
use crate::*;
use crate::{DaemonSDK, DaemonSDKExtra, DaemonSdkSealed, InternalDaemonSDK};
use ed25519_dalek::SigningKey;
use std::ops::Deref;
use std::sync::Arc;
use url::Url;

#[derive(Clone)]
pub struct WithUserKey(pub(crate) SigningKey);

#[derive(Clone)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub struct DaemonSDKWithUserKey(pub(crate) Arc<InternalDaemonSDK<WithUserKey>>);

impl Deref for DaemonSDKWithUserKey {
    type Target = InternalDaemonSDK<WithUserKey>;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl DaemonSDKExtra for WithUserKey {
    type Inner = DaemonSDKWithUserKey;
}
impl DaemonSdkSealed for WithUserKey {}
impl DaemonSDKBuilderState for WithUserKey {
    type Inner = DaemonSDKBuilderWithUserKey;
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub struct DaemonSDKBuilderWithUserKey {
    pub(crate) identifier: String,
    pub(crate) token: String,
    pub(crate) base_url: Url,
    pub(crate) user_key: SigningKey,
}

impl DaemonSDKBuilder<()> {
    pub fn with_user_key(self, user_key: SigningKey) -> DaemonSDKBuilder<WithUserKey> {
        DaemonSDKBuilder(DaemonSDKBuilderWithUserKey {
            identifier: self.0.identifier,
            token: self.0.token,
            base_url: self.0.base_url,
            user_key,
        })
    }
}

impl DaemonSDKBuilder<WithUserKey> {
    pub fn build(self) -> DaemonSDK<WithUserKey> {
        DaemonSDK(DaemonSDKWithUserKey(Self::build0(
            self.0.identifier,
            self.0.token,
            self.0.base_url,
            WithUserKey(self.0.user_key),
        )))
    }
}
