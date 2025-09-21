// SPDX-License-Identifier: AGPL-3.0-only
use super::*;
use zelus::service;

define_error!(network {
    not_found ("The network does not exist" NOT_FOUND),
    prefix_collision ("There is already a network with this ipv4 or ipv6 network prefix" FORBIDDEN),
    prefix_invalid ("One of the given prefixes are invalid" BAD_REQUEST),
});
define_path_variable!(network_id "The network id");

#[service(path = "/network", tag = "network")]
pub trait NetworkService {
    /// Creates a new network
    #[route("", method = PUT)]
    #[error(network(prefix_collision [sql:unique] prefix_invalid [sql:check]) docker(failure) sql(failure [sql:*]))]
    #[example("examples/network.create.json")]
    async fn create_network(&self, ipv4: u16, ipv6: u16) -> Result<u32, _>;

    /// Deletes an existing network
    #[route("/{network_id}", method = DELETE)]
    #[error(network(not_found) docker(failure) sql(failure [sql:*]))]
    async fn delete_network(&self, network_id: u32) -> Result<(), _>;
}
