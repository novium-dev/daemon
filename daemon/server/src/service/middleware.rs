use crate::Daemon;
use axum::Extension;
use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_extra::TypedHeader;
use axum_extra::headers::Authorization;
use axum_extra::headers::authorization::Bearer;
use daemon_common::service::MiddlewareError;

pub(crate) async fn middleware(
    Extension(daemon): Extension<Daemon>,
    header: Option<TypedHeader<Authorization<Bearer>>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(TypedHeader(Authorization(header))) = header else {
        return MiddlewareError::AuthInvalid.into_response();
    };
    if !header.token().eq(&daemon.config.token) {
        return MiddlewareError::AuthInvalid.into_response();
    }
    next.run(request).await
}
