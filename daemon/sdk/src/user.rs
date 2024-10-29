use crate::builder::user::WithUserKey;
use crate::DaemonSDK;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use ed25519_dalek::Signer;
use std::clone::Clone;

#[cfg(feature = "websocket")]
#[derive(Copy, Clone)]
pub enum WebsocketType {
    TERMINAL,
    STATS,
}

impl DaemonSDK<WithUserKey> {
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

    #[cfg(feature = "backups")]
    pub fn generate_backup_download_url(
        &self,
        server: u32,
        timestamp: u64,
        storage: String,
        backup: i64,
    ) -> String {
        let verify = self.generate_token(
            "file",
            server,
            timestamp,
            Some(format!("-{}-{}", storage, backup)),
        );
        let mut url = self
            .base_url
            .join(&format!("user/backup/{}/{}/{}", server, storage, backup))
            .unwrap();
        url.query_pairs_mut()
            .append_pair("timestamp", &timestamp.to_string())
            .append_pair("verify", &verify);
        url.to_string()
    }

    #[cfg(feature = "backups")]
    pub fn generate_realtime_backup_download_url(
        &self,
        server: u32,
        timestamp: u64,
        compression: String,
    ) -> String {
        let verify =
            self.generate_token("file", server, timestamp, Some(format!("-{}", compression)));
        let mut url = self
            .base_url
            .join(&format!("user/backup/{}/rt", server))
            .unwrap();
        url.query_pairs_mut()
            .append_pair("timestamp", &timestamp.to_string())
            .append_pair("verify", &verify)
            .append_pair("compression", &compression);
        url.to_string()
    }
}
