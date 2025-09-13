use daemon_framework::error::BlankError;
use daemon_framework::service;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
struct ExampleResult {
    test: u32,
}

#[service]
trait ExampleService {
    #[route("/", method = GET, no_auth)]
    async fn example(&self) -> Result<ExampleResult, BlankError>;
}

fn main() {}
