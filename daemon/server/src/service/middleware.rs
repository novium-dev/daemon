use crate::config::DaemonConfig;
use axum::Extension;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Bearer;
use daemon_common::service::BlankError;

pub async fn middleware(
    Extension(config): Extension<DaemonConfig>,
    header: Option<TypedHeader<Authorization<Bearer>>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(TypedHeader(Authorization(header))) = header else {
        return BlankError::AuthInvalid.into_response();
    };
    if !header.token().eq(&config.token) {
        return BlankError::AuthInvalid.into_response();
    }
    next.run(request).await
}
