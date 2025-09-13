use daemon_framework::service;
use daemon_framework_macros::define_error;

define_error!(auth {
    invalid ("Authentication is invalid" UNAUTHORIZED)
});

#[service]
trait ExampleService {
    #[route("/", method = GET, no_auth)] // This has no_auth, meaning its in the "without_auth" route and has an empty error
    #[error()]
    async fn example1(&self) -> Result<(), _>;

    #[route("/", method = GET)]
    // This has not "no_auth", meaning its in the "with_auth" route and has an auth(invalid) error automatically added
    #[error()]
    async fn example2(&self) -> Result<(), _>;
}

fn main() {}
