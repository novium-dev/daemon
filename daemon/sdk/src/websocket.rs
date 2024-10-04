#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use ed25519_dalek::Signer;
use crate::InternalDaemonSDK;
use crate::builder::DaemonSDKBuilder;
use crate::builder::websocket::{DaemonSDKBuilderWithWebsocket, DaemonSDKWithWebsocketKey, WithWebsocketKey};

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub enum WebsocketType {
    TERMINAL,
    STATS
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
impl DaemonSDKWithWebsocketKey {

    pub fn generate_websocket_token(&self, server: u32, timestamp: u64) -> String {
        BASE64_STANDARD.encode(self.extra.0.sign(format!("{}-{}-{}", self.identifier, server, timestamp).as_bytes()).to_vec())
    }

    pub fn generate_websocket_url(&self, r#type: WebsocketType, server: u32, timestamp: u64) -> String {
        let verify = self.generate_websocket_token(server, timestamp);
        let mut url = self.base_url.join(&format!("user/server/{}/{}", server, match r#type {
            WebsocketType::TERMINAL => "terminal",
            WebsocketType::STATS => "stats"
        }))
            .unwrap();
        url.set_query(Some(&format!("timestamp={}&verify={}", timestamp, verify)));
        url.to_string()
    }

}
