use axum::http::StatusCode;
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use serde::Deserialize;
use tracing::debug;

use crate::DaemonState;

#[allow(unused)]
#[derive(Deserialize)]
pub(crate) struct GenericUserQuery {
    pub(crate) verify: String,
    pub(crate) timestamp: String,
}

#[allow(unused)]
pub(crate) async fn authenticate_user(
    name: &str,
    #[allow(unused)] server_id: u32,
    #[allow(unused)] state: &DaemonState,
    timestamp: &str,
    verify: &String,
    #[allow(unused)] extra: Option<String>,
) -> Result<(), StatusCode> {
    debug!("Authenticating User...");
    #[allow(unused)]
    let timestamp: u64 = timestamp.parse().map_err(|_err| StatusCode::BAD_REQUEST)?;
    let verify = BASE64_STANDARD
        .decode(verify.as_bytes())
        .inspect_err(|_err| debug!("User verification is invalid. Base64 Invalid"))
        .map_err(|_err| StatusCode::BAD_REQUEST)?;
    #[cfg(feature = "user_debug_auth_bypass")]
    if !verify.eq("bypass".as_bytes()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    #[cfg(not(feature = "user_debug_auth_bypass"))]
    {
        use ed25519_dalek::{Signature, Verifier};
        use std::time::{SystemTime, UNIX_EPOCH};

        let signature_bytes: [u8; 64] =
            verify.try_into().map_err(|_err| StatusCode::BAD_REQUEST)?;
        state
            .config
            .user_key
            .verify(
                format!(
                    "{}-{}-{}-{}{}",
                    name,
                    state.config.identifier,
                    server_id,
                    timestamp,
                    extra.unwrap_or_default()
                )
                .as_bytes(),
                &Signature::from_bytes(&signature_bytes),
            )
            .inspect_err(|_err| debug!("User verification failed. Signature is invalid"))
            .map_err(|_err| StatusCode::UNAUTHORIZED)?;
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .inspect_err(|err| debug!("Time error: {:?}", err))
            .map_err(|_err| StatusCode::BAD_REQUEST)?;
        if (timestamp + state.config.user_allow_duration) as u128 <= unix.as_millis() {
            debug!("User verification failed. Verification expired.");
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    debug!("User authentication passed.");
    Ok(())
}
