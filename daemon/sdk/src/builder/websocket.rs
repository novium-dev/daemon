use crate::*;
use std::ops::Deref;
use std::sync::Arc;
use ed25519_dalek::SigningKey;
use url::Url;
use crate::{DaemonSDK, DaemonSDKExtra, DaemonSdkSealed, InternalDaemonSDK};
use crate::builder::{DaemonSDKBuilder, DaemonSDKBuilderState};

pub struct WithWebsocketKey(pub(crate) SigningKey);

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub struct DaemonSDKWithWebsocketKey(pub(crate) Arc<InternalDaemonSDK<WithWebsocketKey>>);

impl Deref for DaemonSDKWithWebsocketKey {
    type Target = InternalDaemonSDK<WithWebsocketKey>;

    fn deref(&self) -> &Self::Target {
        self.0.deref()
    }
}

impl DaemonSDKExtra for WithWebsocketKey {
    type Inner = DaemonSDKWithWebsocketKey;
}
impl DaemonSdkSealed for WithWebsocketKey {}
impl DaemonSDKBuilderState for WithWebsocketKey {
    type Inner = DaemonSDKBuilderWithWebsocket;
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub struct DaemonSDKBuilderWithWebsocket {
    identifier: String,
    token: String,
    base_url: Url,
    websocket_key: SigningKey,
}

impl DaemonSDKBuilder<()> {

    pub fn with_websocket_key(self, websocket_key: SigningKey) -> Result<DaemonSDKBuilder<WithWebsocketKey>, reqwest::Error> {
        Ok(DaemonSDKBuilder(DaemonSDKBuilderWithWebsocket {
            identifier: self.0.identifier,
            token: self.0.token,
            base_url: self.0.base_url,
            websocket_key
        }))
    }

}

impl DaemonSDKBuilder<WithWebsocketKey> {
    pub fn build(self) -> DaemonSDK<WithWebsocketKey> {
        DaemonSDK(DaemonSDKWithWebsocketKey(Self::build0(self.0.identifier, self.0.token, self.0.base_url, WithWebsocketKey(self.0.websocket_key))))
    }
}