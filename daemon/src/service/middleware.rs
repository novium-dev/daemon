// SPDX-License-Identifier: AGPL-3.0-only
use axum::Extension;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};
use daemon_common::auth::NodeToken;
use daemon_common::service::BlankError;
use rustls::RootCertStore;
use std::sync::Arc;
use tracing::debug;
use uuid::Uuid;

/// What the authentication middleware needs to validate a request.
///
/// This replaces the peer certificate that mutual TLS used to attach to the
/// connection, so authentication no longer depends on the TLS layer surviving a
/// reverse proxy.
#[derive(Debug, Clone)]
pub struct AuthData {
    pub cert_store: Arc<RootCertStore>,
    pub node: Uuid,
}

pub async fn middleware(
    Extension(AuthData { cert_store, node }): Extension<AuthData>,
    request: Request,
    next: Next,
) -> Response {
    let Some(token) = request
        .headers()
        .get(daemon_common::DAEMON_AUTH_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        debug!("Rejecting request without an authentication token");
        return BlankError::AuthInvalid.into_response();
    };

    let token = match token.parse::<NodeToken>() {
        Ok(token) => token,
        Err(err) => {
            debug!("Rejecting request with an unparsable token: {err}");
            return BlankError::AuthInvalid.into_response();
        }
    };

    if let Err(err) = token.verify(&cert_store, node) {
        debug!("Rejecting request with an invalid token: {err}");
        return BlankError::AuthInvalid.into_response();
    }

    next.run(request).await
}
