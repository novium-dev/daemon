use daemon_framework::error::BlankError;
use daemon_framework::{framework_router, service};
use daemon_framework_macros::define_error;
use tokio::net::TcpListener;

#[derive(Clone)]
struct Base;

define_error!(auth {
    invalid ("Authentication is invalid" UNAUTHORIZED)
});

#[service]
impl ExampleService for Base {
    #[route("/route1", method = GET, no_auth, routes = [ example1 ])] // This will be available at port 3000, and 3004 because "no_auth"
    async fn example1(&self) -> Result<(), BlankError> {
        // implementation here
        Ok(())
    }

    #[route("/route2", method = GET, routes = [ example2 ])] // This will be available at port 3001, and 3003 because "no_auth" is not set
    async fn example2(&self) -> Result<(), BlankError> {
        // implementation here
        Ok(())
    }

    #[route("/route3", method = GET, no_auth)] // This will be available at port 3002, as if unspecified its `routes = [ default ]`, and 3004 because "no_auth"
    async fn example3(&self) -> Result<(), BlankError> {
        // implementation here
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let base = Base;

    // with_auth and without_auth cannot be overwriten and always exists.

    let (example1, example2, default, with_auth, without_auth) = framework_router!(Base base (example1,example2,default,with_auth,without_auth,) { ExampleService });
    tokio::task::spawn(async move {
        axum::serve(
            TcpListener::bind("[::]:3001")
                .await
                .expect("Unable to bind"),
            example2
                .into_openapi()
                .split_for_parts()
                .0
                .into_make_service(),
        )
        .await
        .expect("Unable to serve http server");
    });
    tokio::task::spawn(async move {
        axum::serve(
            TcpListener::bind("[::]:3002")
                .await
                .expect("Unable to bind"),
            default
                .into_openapi()
                .split_for_parts()
                .0
                .into_make_service(),
        )
        .await
        .expect("Unable to serve http server");
    });
    tokio::task::spawn(async move {
        axum::serve(
            TcpListener::bind("[::]:3003")
                .await
                .expect("Unable to bind"),
            with_auth
                .into_openapi()
                .split_for_parts()
                .0
                .into_make_service(),
        )
        .await
        .expect("Unable to serve http server");
    });
    tokio::task::spawn(async move {
        axum::serve(
            TcpListener::bind("[::]:3004")
                .await
                .expect("Unable to bind"),
            without_auth
                .into_openapi()
                .split_for_parts()
                .0
                .into_make_service(),
        )
        .await
        .expect("Unable to serve http server");
    });
    axum::serve(
        TcpListener::bind("[::]:3000")
            .await
            .expect("Unable to bind"),
        example1
            .into_openapi()
            .split_for_parts()
            .0
            .into_make_service(),
    )
    .await
    .expect("Unable to serve http server");
}
