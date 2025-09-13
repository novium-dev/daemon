use daemon_framework::service;

#[service]
trait ExampleService {
    #[route("/", method = GET, no_auth)]
    #[error({ Custom "This is a custom error" IM_A_TEAPOT })]
    async fn example(&self) -> Result<(), _>;
}

fn main() {}
