use axum::extract::FromRequestParts;
use daemon_framework::error::BlankError;
use daemon_framework::sdk::IntoRequestParts;
use daemon_framework::service;
use daemon_framework::types::DocumentedType;
use http::request::Parts;
use reqwest::RequestBuilder;

#[expect(dead_code)]
struct SpecialVariable;

impl<S: Send + Sync> FromRequestParts<S> for SpecialVariable {
    type Rejection = ();

    async fn from_request_parts(_parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self)
    }
}

impl DocumentedType for SpecialVariable {} // If you want to modify documentation with this type

// This is not required, if you set "no_sdk" in the service options
impl IntoRequestParts for SpecialVariable {
    async fn into_request(self, req: RequestBuilder) -> RequestBuilder {
        req
    }
}

#[service]
trait ExampleService {
    #[route("/", method = GET, no_auth)]
    async fn example(&self, #[special] special_variable: SpecialVariable)
    -> Result<(), BlankError>;
}

fn main() {}
