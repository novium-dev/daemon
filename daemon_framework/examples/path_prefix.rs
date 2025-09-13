use daemon_framework::error::BlankError;
use daemon_framework::service;

#[service(path = "/example")] // These are appended onto every path
trait ExampleService {
    #[route("", method = GET, no_auth)] // Actual path: "/example"
    async fn example1(&self) -> Result<(), BlankError>;

    #[route(!"/", method = GET, no_auth)] // This has an ! infront of the path. The "/example" will not be appended here. Actual Path: "/"
    async fn example2(&self) -> Result<(), BlankError>;
}

fn main() {}
