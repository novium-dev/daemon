use daemon_framework::error::BlankError;
use daemon_framework::service;

#[service]
trait ExampleService {
    /// This is documentation for this route
    /// It will be included in openapi
    #[route("/", method = GET, no_auth)]
    async fn example(&self) -> Result<(), BlankError>;
}

fn main() {}
