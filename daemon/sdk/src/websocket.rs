use ed25519_dalek::SigningKey;
use crate::{DaemonSDKBuilder, DaemonSDKBuilderState, DaemonSDKExtra};

pub struct WithWebsocketKey(pub(crate) SigningKey);

impl DaemonSDKExtra for WithWebsocketKey {}
impl DaemonSDKBuilderState for WithWebsocketKey {}

#[cfg(feature = "websocket")]
impl DaemonSDKBuilder<()> {

    pub fn with_websocket_key(self, websocket_key: SigningKey) -> Result<DaemonSDKBuilder<WithWebsocketKey>, reqwest::Error> {
        Ok(DaemonSDKBuilder {
            identifier: self.identifier,
            token: self.token,
            base_url: self.base_url,
            extra: Some(WithWebsocketKey(websocket_key)),
        })
    }

}