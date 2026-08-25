// SPDX-License-Identifier: AGPL-3.0-only
//! Node identity attestation.
//!
//! Before the reverse proxy existed, the backend learned which node it was
//! talking to from the TLS certificate, by pinning the server name to
//! `node-<uuid>`. A proxy terminates TLS and presents its own certificate, so
//! that signal is gone. This endpoint restores it at the application layer: the
//! backend sends a random challenge and the node signs it with its node key.
//!
//! The route is deliberately unauthenticated. It only ever reveals the node's
//! own certificate plus a signature over a caller supplied challenge, which the
//! node already presents to anyone completing a direct TLS handshake.

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use daemon_common::auth::{Attestation, Challenge};
use daemon_common::key::DaemonKey;
use serde::Deserialize;
use std::sync::Arc;
use tracing::warn;

#[derive(Debug, Deserialize)]
pub struct AttestQuery {
    challenge: String,
}

pub fn router(key: Arc<DaemonKey>) -> Router {
    Router::new()
        .route(
            &format!("/{}", daemon_common::DAEMON_ATTEST_PATH),
            get(attest),
        )
        .with_state(key)
}

async fn attest(
    State(key): State<Arc<DaemonKey>>,
    Query(AttestQuery { challenge }): Query<AttestQuery>,
) -> Response {
    let Ok(challenge) = challenge.parse::<Challenge>() else {
        return StatusCode::BAD_REQUEST.into_response();
    };

    let attestation = match Attestation::sign(&key.tls_cert, key.id, challenge) {
        Ok(attestation) => attestation,
        Err(err) => {
            warn!("Unable to sign attestation: {err}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let Ok(value) = HeaderValue::from_str(&attestation.to_string()) else {
        warn!("Attestation is not a valid header value");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    (
        StatusCode::NO_CONTENT,
        [(daemon_common::DAEMON_ATTESTATION_HEADER, value)],
    )
        .into_response()
}
