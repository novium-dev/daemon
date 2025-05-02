use super::*;
use crate::payload::resource::ResourceResponse;
use daemon_framework::service;

#[service(tag = "resource")]
pub trait ResourceService {
    #[route("/resource", method = GET)]
    #[error(sql(failure [sql:*]) { ResourceError "Unable to retrieve the resource info" INTERNAL_SERVER_ERROR })]
    #[doc = "Gets resource information"]
    async fn resource_info(&self) -> Result<ResourceResponse, _>;
}
