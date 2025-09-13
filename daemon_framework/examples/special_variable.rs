use crate::error_special_test::FromSpecialTest;
use daemon_framework::service;
use daemon_framework::special::{FromRequestParts, IntoRequestParts};
use daemon_framework::types::DocumentedType;
use daemon_framework_macros::define_error;
use http::request::Parts;
use reqwest::RequestBuilder;

#[expect(dead_code)]
struct SpecialVariable;

define_error!(special {
    test ("Test error" IM_A_TEAPOT)
});

impl<E: FromSpecialTest> FromRequestParts<E> for SpecialVariable {
    async fn from_request_parts(_parts: &mut Parts) -> Result<Self, E> {
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
    #[error(special(test))]
    async fn example(&self, #[special] special_variable: SpecialVariable) -> Result<(), _>;
}

fn main() {}
