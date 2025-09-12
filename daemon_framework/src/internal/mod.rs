mod error;
mod header;
mod json;
mod serializer;

pub use error::{error_by_response, error_schema};
pub use header::{header_insert, header_name};
pub use json::FrameworkJsonResponse;
pub use serializer::StringSerializer;

use crate::SUCCESS_DESCRIPTION;
use crate::responses::DocumentedResponse;
use crate::utils::MaybeUnit;
use std::collections::HashMap;
use utoipa::ToSchema;
use utoipa::openapi::schema::RefBuilder;
use utoipa::openapi::{Content, RefOr, Response, ResponsesBuilder, Schema};

#[must_use]
pub fn from_raw<T: From<reqwest::Response>>(response: reqwest::Response) -> T {
    T::from(response)
}

impl<V: ToSchema + MaybeUnit + 'static, E: DocumentedResponse + 'static> DocumentedResponse
    for Result<V, E>
{
    fn error_values() -> &'static [&'static str] {
        E::error_values()
    }

    fn openapi(
        mut responses: ResponsesBuilder,
        schemas: &mut HashMap<String, RefOr<Schema>>,
    ) -> ResponsesBuilder {
        if V::unit().is_some() {
            responses = responses.response(
                "204",
                Response::builder().description(SUCCESS_DESCRIPTION).build(),
            );
        } else {
            responses = responses.response(
                "200",
                Response::builder()
                    .description(SUCCESS_DESCRIPTION)
                    .content(
                        "application/json",
                        Content::new(Some(
                            RefBuilder::new().ref_location_from_schema_name(V::name()),
                        )),
                    )
                    .build(),
            );
            let mut vals = Vec::new();
            vals.push((V::name().to_string(), V::schema()));
            V::schemas(&mut vals);
            schemas.extend(vals);
        }

        E::openapi(responses, schemas)
    }
}
