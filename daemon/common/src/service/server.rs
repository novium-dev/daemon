use super::*;
use crate::payload::server::{
    ServerBuildPayload, ServerNetworkPayload, ServerPortEntryPayload, ServerResourcePayload,
};
use crate::server::PublicServerState;
use crate::user::*;
use chrono::Utc;
use daemon_framework::{JsonVec, WebsocketResponse, define_query_variable};
use url::Url;

define_error!(server {
    image_different ("Server image differs in database" BAD_REQUEST),
    image_missing_tag ("Server image does not have a tag" BAD_REQUEST),
});

define_query_variable!(write "If write access to the terminal should be granted");
define_query_variable!(cols "The initial value of columns in the terminal");
define_query_variable!(rows "The initial value of rows in the terminal");
define_query_variable!(command "The command to be executed encoded as json string array");

define_query_variable!(precise "If stopping or restarting should be determined. Default: true. If false, the server mutex does not have to be locked.");

#[service(path = "/server", tag = "server")]
pub trait ServerService {
    #[route("", method = PUT)]
    #[error(server(image_different image_missing_tag) storage(location_not_found) sql(failure [sql:*]))]
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
    #[doc = "Executes a command inside an running server"]
    async fn exec_server(&self, server_id: u32, cmd: Vec<String>);

    // TODO: Move to FilesService
    #[route("/{server_id}/wipe", method = GET)]
    #[error(storage(not_found mutex_locked failure [io:*]) sql(failure [sql:*]))]
    #[doc = "Deletes all files in an existing server"]
    async fn wipe_server(&self, server_id: u32);

    #[route(!"/user/server/{server_id}/terminal", method = WS, query = [ timestamp, verify, cols, rows, write ], user)]
    #[error(auth(signature_invalid signature_expired) server(not_found) sql(failure [sql:*]))]
    #[doc = "A websocket for interaction with the terminal"]
    async fn server_terminal(
        &self,
        server_id: u32,
        cols: Option<u32>,
        rows: Option<u32>,
        write: bool,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, _>;

    #[route(!"/user/server/{server_id}/stats", method = WS, query = [ timestamp, verify ], user)]
    #[error(auth(signature_invalid signature_expired) server(not_found) sql(failure [sql:*]))]
    #[doc = "A websocket for server stats"]
    async fn server_stats(
        &self,
        server_id: u32,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, _>;

    #[route(!"/user/server/{server_id}/exec", method = WS, query = [ timestamp, verify, cols, rows, command ], user)]
    #[error(auth(signature_invalid signature_expired) server(not_found) sql(failure [sql:*]))]
    #[doc = "A websocket for server execution"]
    async fn exec_server_interactive(
        &self,
        server_id: u32,
        cols: Option<u32>,
        rows: Option<u32>,
        command: JsonVec<String>,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, _>;
}

pub trait ServerServiceUserKeyExtension {
    fn server_terminal_url_signed(
        &self,
        server_id: u32,
        cols: Option<u32>,
        rows: Option<u32>,
        write: bool,
    ) -> Url;

    fn server_stats_url_signed(&self, server_id: u32) -> Url;

    fn exec_server_interactive_url_signed(
        &self,
        server_id: u32,
        cols: Option<u32>,
        rows: Option<u32>,
        command: JsonVec<String>,
    ) -> Url;

    #[cfg(feature = "ts")]
    fn visit_funcs(visitor: &mut daemon_framework::ts::TSClassFunctionVisitor);
}

impl<T: ServerServiceURL + UserKeyProvider> ServerServiceUserKeyExtension for T {
    fn server_terminal_url_signed(
        &self,
        server_id: u32,
        cols: Option<u32>,
        rows: Option<u32>,
        write: bool,
    ) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature =
            UserSignature::new_terminal(self.key(), timestamp, self.node_id(), server_id, write);
        self.server_terminal_url(server_id, cols, rows, write, timestamp, signature)
    }

    fn server_stats_url_signed(&self, server_id: u32) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature = UserSignature::new_stats(self.key(), timestamp, self.node_id(), server_id);
        self.server_stats_url(server_id, timestamp, signature)
    }

    fn exec_server_interactive_url_signed(
        &self,
        server_id: u32,
        cols: Option<u32>,
        rows: Option<u32>,
        command: JsonVec<String>,
    ) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature =
            UserSignature::new_exec(self.key(), timestamp, self.node_id(), server_id, &command);
        self.exec_server_interactive_url(server_id, cols, rows, command, timestamp, signature)
    }

    #[cfg(feature = "ts")]
    fn visit_funcs(visitor: &mut daemon_framework::ts::TSClassFunctionVisitor) {
        visitor.visit_start(true, "serverTerminalUrlSigned");
        visitor.visit_argument::<u32>("serverId");
        visitor.visit_argument::<Option<u32>>("cols");
        visitor.visit_argument::<Option<u32>>("rows");
        visitor.visit_argument::<bool>("write");
        visitor.visit_end_result_custom(r#"const timestamp: bigint = BigInt(Date.now());
const signature = await UserSignature.newTerminalAsync(this.userKey, timestamp, this.identifier, serverId, write);
return this.serverTerminalUrl(serverId, cols, rows, write, timestamp, signature);"#, "URL");

        visitor.visit_start(true, "serverStatsUrlSigned");
        visitor.visit_argument::<u32>("serverId");
        visitor.visit_end_result_custom(r#"const timestamp: bigint = BigInt(Date.now());
const signature = await UserSignature.newStatsAsync(this.userKey, timestamp, this.identifier, serverId);
return this.serverStatsUrl(serverId, timestamp, signature);"#, "URL");

        visitor.visit_start(true, "execServerInteractiveUrlSigned");
        visitor.visit_argument::<u32>("serverId");
        visitor.visit_argument::<Option<u32>>("cols");
        visitor.visit_argument::<Option<u32>>("rows");
        visitor.visit_argument::<JsonVec<String>>("command");
        visitor.visit_end_result_custom(r#"const timestamp: bigint = BigInt(Date.now());
const signature = await UserSignature.newExecAsync(this.userKey, timestamp, this.identifier, serverId, command);
return this.execServerInteractiveUrl(serverId, cols, rows, command, timestamp, signature);"#, "URL");
    }
}
