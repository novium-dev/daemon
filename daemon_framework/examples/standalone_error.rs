use daemon_framework::service;
use daemon_framework_macros::{define_error, error};

define_error!(category {
    error1 ("This is the error message for error1" IM_A_TEAPOT),
    error2 ("This is the error message for error2" IM_A_TEAPOT),
});

error!(StandaloneError, category(error1 error2));

#[service]
trait ExampleService1 {
    #[route("/", method = GET, no_auth)]
    async fn example1(&self) -> Result<(), StandaloneError>;
}

#[service]
trait ExampleService2 {
    #[route("/", method = GET, no_auth)]
    async fn example2(&self) -> Result<(), StandaloneError>;
}

fn main() {}
