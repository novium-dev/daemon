mod file;
mod ws;

use std::collections::HashMap;
use utoipa::openapi::{RefOr, Schema};

pub use file::FileResponse;
pub use ws::WebsocketResponse;

pub trait DocumentedResponse {
    fn error_values() -> &'static [&'static str];

    fn openapi(
        responses: utoipa::openapi::ResponsesBuilder,
        schemas: &mut HashMap<String, RefOr<Schema>>,
    ) -> utoipa::openapi::ResponsesBuilder;
}
