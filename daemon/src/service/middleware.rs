/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
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
