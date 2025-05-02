use super::*;
use crate::payload::database::{DatabaseUserPayload, DatabaseUserPermissions};
use daemon_framework::service;
use std::collections::HashMap;

define_error!(database {
    not_found ("The database does not exist" NOT_FOUND),
    invalid_identifier ("The sql identifier is invalid" BAD_REQUEST),
    invalid_hash ("The sql password hash is invalid" BAD_REQUEST),
    permissionless_entry ("One permission entry doesn't have privileges" BAD_REQUEST),
});
define_path_variable!(database_id "The database id");

#[service(path = "/database", tag = "database")]
pub trait DatabaseService {
    #[route("", method = PUT)]
    #[error(database(invalid_identifier) sql(failure [sql:*]))]
    #[example("examples/database.create.json")]
    #[doc = "Creates a new database"]
    async fn create_database(&self, name_suffix: String) -> Result<u32, _>;

    #[route("/{database_id}", method = PATCH)]
    #[error(database(not_found invalid_identifier invalid_hash permissionless_entry) sql(failure [sql:*]))]
    #[example("examples/database.update.json")]
    #[doc = "Updates an existing database"]
    async fn update_database(
        &self,
        database_id: u32,
        users: HashMap<String, DatabaseUserPayload>,
        passwordless: HashMap<u32, DatabaseUserPermissions>,
    ) -> Result<(), _>;

    #[route("/{database_id}", method = DELETE)]
    #[error(database(not_found) sql(failure [sql:*]))]
    #[doc = "Updates an existing database"]
    async fn delete_database(&self, database_id: u32) -> Result<(), _>;
}
