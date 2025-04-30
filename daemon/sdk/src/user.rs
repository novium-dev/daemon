use crate::DaemonSDK;
use crate::builder::user::WithUserKey;
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use ed25519_dalek::Signer;
use std::clone::Clone;
use url::Url;

#[cfg(feature = "user")]
#[derive(Copy, Clone)]
pub enum WebsocketType {
    TERMINAL,
    STATS,
}

impl DaemonSDK<WithUserKey> {
    #[deprecated]
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

    #[deprecated]
    pub fn generate_file_download_url(&self, server: u32, timestamp: u64, path: String) -> Url {
        let verify = self.generate_token("file", server, timestamp, Some(format!("-{}", path)));
        let mut url = self
            .base_url
            .join(&format!("user/files/{}/file", server))
            .unwrap();
        url.query_pairs_mut()
            .append_pair("timestamp", &timestamp.to_string())
            .append_pair("verify", &verify)
            .append_pair("path", &path);
        url
    }
}
