use daemon_framework::service;
use daemon_framework_macros::define_error;

define_error!(category {
    error1 ("This is the error message for error1" IM_A_TEAPOT),
    error2 ("This is the error message for error2" IM_A_TEAPOT),
});

#[service]
trait ExampleService1 {
    #[route("/", method = GET, no_auth)]
    #[error(category(error1 error2))]
    async fn example1(&self) -> Result<(), _>;
}

#[service]
trait ExampleService2 {
    #[route("/", method = GET, no_auth)]
    #[error(category(error2))]
    async fn example2(&self) -> Result<(), _>;
}

fn main() {}
