use super::*;
use crate::payload::server::{
    ServerBuildPayload, ServerNetworkPayload, ServerPortEntryPayload, ServerResourcePayload,
};
use crate::server::PublicServerState;
use daemon_framework::define_query_variable;

define_error!(server {
    image_different ("Server image differs in database" BAD_REQUEST),
    image_missing_tag ("Server image does not have a tag" BAD_REQUEST),
});

define_query_variable!(precise "If stopping or restarting should be determined. Default: true. If false, the server mutex does not have to be locked.");

#[service(path = "/server", tag = "server")]
pub trait ServerService {
    #[route("", method = PUT)]
    #[error(server(image_different image_missing_tag) storage(not_found) sql(failure [sql:*]))]
    #[example("examples/server.create.json")]
    #[doc = "Creates a new server"]
    async fn create_server(
        &self,
        storage: String,
        resource: ServerResourcePayload,
        build: ServerBuildPayload,
        ports: Vec<ServerPortEntryPayload>,
        network: ServerNetworkPayload,
    ) -> Result<u32, _>;

    #[route("/{server_id}", method = PATCH)]
    #[error(server(not_found image_different image_missing_tag) sql(failure [sql:*]))]
    #[example("examples/server.update.json")]
    #[doc = "Updates an existing server"]
    async fn update_server(
        &self,
        server_id: u32,
        resource: Option<ServerResourcePayload>,
        build: Option<ServerBuildPayload>,
        ports: Option<Vec<ServerPortEntryPayload>>,
        network: Option<ServerNetworkPayload>,
    );

    #[route("/{server_id}", method = DELETE)]
    #[error(server(not_found mutex_locked) docker(failure) sql(failure [sql:*]))]
    #[doc = "Deletes an existing server"]
    async fn delete_server(&self, server_id: u32);

    #[route("/{server_id}", method = GET, query = [ precise ])]
    #[error(server(not_found mutex_locked) docker(failure) sql(failure [sql:*]))]
    #[doc = "Gets the status of an existing server"]
    async fn server_status(
        &self,
        server_id: u32,
        precise: Option<bool>,
    ) -> Result<PublicServerState, _>;

    #[route("/{server_id}/start", method = GET)]
    #[error(server(not_found) sql(failure [sql:*]))]
    #[doc = "Starts an existing server"]
    async fn start_server(&self, server_id: u32);

    #[route("/{server_id}/stop", method = GET)]
    #[error(server(not_found) sql(failure [sql:*]))]
    #[doc = "Stops an existing server"]
    async fn stop_server(&self, server_id: u32);

    #[route("/{server_id}/kill", method = GET)]
    #[error(server(not_found) sql(failure [sql:*]))]
    #[doc = "Kills an existing server"]
    async fn kill_server(&self, server_id: u32);

    #[route("/{server_id}/restart", method = GET)]
    #[error(server(not_found) sql(failure [sql:*]))]
    #[doc = "Restarts an existing server"]
    async fn restart_server(&self, server_id: u32);

    #[route("/{server_id}/exec", method = POST)]
    #[error(server(not_found mutex_locked) sql(failure [sql:*]))]
    #[example("examples/server.exec.json")]
    #[doc = "Executes a command in an running server"]
    async fn exec_server(&self, server_id: u32, cmd: Vec<String>);

    #[route("/{server_id}/wipe", method = GET)]
    #[error(server(not_found) storage(failure [io:*] mutex_locked) sql(failure [sql:*]))]
    #[doc = "Deletes all files in an existing server"]
    async fn wipe_server(&self, server_id: u32);
}
