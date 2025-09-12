use crate::responses::DocumentedResponse;
use axum::response::IntoResponse;
use std::collections::HashMap;
use utoipa::openapi::{RefOr, ResponsesBuilder, Schema};

pub struct WebsocketResponse(pub axum::response::Response);

impl IntoResponse for WebsocketResponse {
    fn into_response(self) -> axum::response::Response {
        self.0.into_response()
    }
}

impl<E: DocumentedResponse + 'static> DocumentedResponse for Result<WebsocketResponse, E> {
    fn error_values() -> &'static [&'static str] {
        E::error_values()
    }

    fn openapi(
        responses: ResponsesBuilder,
        schemas: &mut HashMap<String, RefOr<Schema>>,
    ) -> ResponsesBuilder {
        <Result<(), E>>::openapi(responses, schemas)
    }
}
