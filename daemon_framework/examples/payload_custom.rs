use daemon_framework::error::BlankError;
use daemon_framework::service;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
struct PayloadStruct {
    field1: u32,
    field2: String,
}

#[service]
trait ExampleService {
    #[route("/", method = POST, no_auth)]
    #[example("payload.json")] // This is optional. It loads the request example
    async fn example(&self, payload: PayloadStruct) -> Result<(), BlankError>; // When there is only one payload argument, it is used as base
}

fn main() {}
