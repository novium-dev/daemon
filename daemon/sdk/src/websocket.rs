#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

use crate::builder::websocket::DaemonSDKWithWebsocketKey;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use ed25519_dalek::Signer;

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
#[derive(Copy, Clone)]
pub enum WebsocketType {
    TERMINAL,
    STATS,
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
impl DaemonSDKWithWebsocketKey {
    fn generate_websocket_token(
        &self,
        r#type: WebsocketType,
        server: u32,
        timestamp: u64,
        write: bool,
    ) -> String {
        BASE64_STANDARD.encode(
            self.extra
                .0
                .sign(
                    format!(
                        "{}-{}-{}-{}{}",
                        match r#type {
                            WebsocketType::TERMINAL => "terminal",
                            WebsocketType::STATS => "stats",
                        },
                        self.identifier,
                        server,
                        timestamp,
                        match r#type {
                            WebsocketType::TERMINAL => format!("-{}", write),
                            _ => String::new(),
                        }
                    )
                    .as_bytes(),
                )
                .to_vec(),
        )
    }

    pub fn generate_websocket_url(
        &self,
        r#type: WebsocketType,
        server: u32,
        timestamp: u64,
        write: Option<bool>,
    ) -> String {
        let write = write.unwrap_or_default();
        let verify = self.generate_websocket_token(r#type, server, timestamp, write);
        let mut url = self
            .base_url
            .join(&format!(
                "user/server/{}/{}",
                server,
                match r#type {
                    WebsocketType::TERMINAL => "terminal",
                    WebsocketType::STATS => "stats",
                }
            ))
            .unwrap();
        url.set_query(Some(&format!(
            "timestamp={}&verify={}{}",
            timestamp,
            verify,
            match r#type {
                WebsocketType::TERMINAL => format!("&write={}", write),
                _ => String::new(),
            }
        )));
        url.to_string()
    }
}
