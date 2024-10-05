use crate::builder::{DaemonSDKBuilder, DaemonSDKBuilderState};
#[cfg(target_arch = "wasm32")]
use crate::*;
use crate::{DaemonSDK, DaemonSDKExtra, DaemonSdkSealed, InternalDaemonSDK};
use ed25519_dalek::SigningKey;
use std::ops::Deref;
use std::sync::Arc;
use url::Url;

pub struct WithWebsocketKey(pub(crate) SigningKey);

#[derive(Clone)]
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
    pub(crate) identifier: String,
    pub(crate) token: String,
    pub(crate) base_url: Url,
    pub(crate) websocket_key: SigningKey,
}

impl DaemonSDKBuilder<()> {
    pub fn with_websocket_key(
        self,
        websocket_key: SigningKey,
    ) -> DaemonSDKBuilder<WithWebsocketKey> {
        DaemonSDKBuilder(DaemonSDKBuilderWithWebsocket {
            identifier: self.0.identifier,
            token: self.0.token,
            base_url: self.0.base_url,
            websocket_key,
        })
    }
}

impl DaemonSDKBuilder<WithWebsocketKey> {
    pub fn build(self) -> DaemonSDK<WithWebsocketKey> {
        DaemonSDK(DaemonSDKWithWebsocketKey(Self::build0(
            self.0.identifier,
            self.0.token,
            self.0.base_url,
            WithWebsocketKey(self.0.websocket_key),
        )))
    }
}
