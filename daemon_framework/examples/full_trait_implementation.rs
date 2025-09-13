use daemon_framework::error::BlankError;
use daemon_framework::{framework_router, service};
use tokio::net::TcpListener;

#[derive(Clone)]
struct Base;

#[service]
trait ExampleService {
    #[route("/", method = GET, no_auth)]
    async fn example(&self) -> Result<(), BlankError>;
}

#[daemon_framework::async_trait]
impl ExampleService for Base {
    async fn example(&self) -> Result<(), BlankError> {
        // implementation here
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let base = Base;
    let (router,) = framework_router!(Base base (default,) { ExampleService });
    axum::serve(
        TcpListener::bind("[::]:3000")
            .await
            .expect("Unable to bind"),
        router
            .into_openapi()
            .split_for_parts()
            .0
            .into_make_service(),
    )
    .await
    .expect("Unable to serve http server");
}
