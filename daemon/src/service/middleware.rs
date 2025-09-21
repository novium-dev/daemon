// SPDX-License-Identifier: AGPL-3.0-only
use crate::key::TlsData;
use axum::Extension;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};
use daemon_common::service::BlankError;

pub async fn middleware(
    Extension(TlsData { peer_certificates }): Extension<TlsData>,
    request: Request,
    next: Next,
) -> Response {
    if peer_certificates.is_none() {
        return BlankError::AuthInvalid.into_response();
    }
    next.run(request).await
}
