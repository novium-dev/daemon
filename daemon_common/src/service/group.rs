// SPDX-License-Identifier: AGPL-3.0-only
use super::*;
use zelus::service;

define_path_variable!(group_name "A group name");

define_error!(group {
    invalid_name ( "Group name does not match regex" BAD_REQUEST ),
});

#[service(path = "/group/{group_name}", tag = "group")]
pub trait GroupService {
    /// Configure group.
    ///
    /// Every group name must match the regex `^([A-Za-z0-9_]+(-[A-Za-z0-9_]+){0,15}){1,200}$`.
    /// If the group does not exist, it will be created.
    /// A group will be automatically deleted if it has no server and no custom configuration.
    #[route("", method = PATCH)]
    #[error(group::invalid_name <- [sql:check], sql::failure <- [sql:*])]
    #[example("examples/group.configure.json")]
    async fn configure_group(
        &self,
        group_name: String,
        cpu: Option<f32>,
        ram: Option<u32>,
        swap: Option<u32>,
    ) -> Result<(), _>;

    /// Delete a group.
    ///
    /// This is equal to removing resource constraints and removing this group from all known
    /// servers.
    #[route("", method = DELETE)]
    #[error(sql::failure <- [sql:*])]
    async fn delete_group(&self, group_name: String) -> Result<(), _>;
}
