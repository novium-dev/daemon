use daemon_framework::service;
use daemon_framework_macros::define_error;
use std::io;
use std::io::ErrorKind;

#[derive(Clone)]
#[expect(dead_code)]
struct Base;

define_error!(category {
    error1 ("This is the error message for error1" IM_A_TEAPOT),
    error2 ("This is the error message for error2" IM_A_TEAPOT),
});

#[service]
impl ExampleService for Base {
    #[route("/", method = GET, no_auth)]
    #[error(category(error1 [io:*] error2 [io:NotFound]))]
    async fn example(&self) -> Result<(), _> {
        if false {
            Err(io::Error::new(ErrorKind::NotFound, "io not found error").into()) // This will be converted into a ExampleError::Error2
        } else {
            Err(io::Error::other("io error").into()) // This will be converted into a ExampleError::Error1
        }
    }
}

fn main() {}
