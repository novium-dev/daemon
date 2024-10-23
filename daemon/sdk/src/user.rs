use std::clone::Clone;
use std::ops::Deref;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

use crate::builder::user::{DaemonSDKWithUserKey, WithUserKey};
use crate::DaemonSDK;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use ed25519_dalek::Signer;

#[cfg(feature = "websocket")]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
#[derive(Copy, Clone)]
pub enum WebsocketType {
    TERMINAL,
    STATS,
}

impl Deref for DaemonSDK<WithUserKey> {
    type Target = DaemonSDKWithUserKey;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
impl DaemonSDKWithUserKey {
    fn generate_token(
        &self,
        name: &str,
        server: u32,
        timestamp: u64,
        extra: Option<String>,
    ) -> String {
        BASE64_STANDARD.encode(
            self.extra
                .0
                .sign(
                    format!(
                        "{}-{}-{}-{}{}",
                        name,
                        self.identifier,
                        server,
                        timestamp,
                        extra.unwrap_or_default()
                    )
                    .as_bytes(),
                )
                .to_vec(),
        )
    }

    #[cfg(feature = "websocket")]
    pub fn generate_websocket_url(
        &self,
        r#type: WebsocketType,
        server: u32,
        timestamp: u64,
        write: Option<bool>,
        rows: Option<u16>,
        cols: Option<u16>,
    ) -> String {
        let verify = self.generate_token(
            match r#type {
                WebsocketType::TERMINAL => "terminal",
                WebsocketType::STATS => "stats",
            },
            server,
            timestamp,
            match r#type {
                WebsocketType::TERMINAL => Some(format!("-{}", write.unwrap())),
                WebsocketType::STATS => None,
            },
        );
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
        url.query_pairs_mut()
            .append_pair("timestamp", &timestamp.to_string())
            .append_pair("verify", &verify);
        if let WebsocketType::TERMINAL = r#type {
            url.query_pairs_mut()
                .append_pair("write", &write.unwrap().to_string())
                .append_pair("cols", &cols.unwrap().to_string())
                .append_pair("rows", &rows.unwrap().to_string());
        }
        url.to_string()
    }

    #[cfg(feature = "files")]
    pub fn generate_file_download_url(&self, server: u32, timestamp: u64, path: String) -> String {
        let verify = self.generate_token("file", server, timestamp, Some(format!("-{}", path)));
        let mut url = self
            .base_url
            .join(&format!("user/files/{}/file", server))
            .unwrap();
        url.query_pairs_mut()
            .append_pair("timestamp", &timestamp.to_string())
            .append_pair("verify", &verify)
            .append_pair("path", &path);
        url.to_string()
    }
}
