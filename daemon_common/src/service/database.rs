// SPDX-License-Identifier: AGPL-3.0-only
use super::*;
use bitflags::bitflags;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::collections::HashMap;
use utoipa::openapi::{RefOr, Schema};
use utoipa::{PartialSchema, ToSchema};
use zelus::service;

define_error!(database {
    not_found ("The database does not exist" NOT_FOUND),
    invalid_identifier ("The sql identifier is invalid" BAD_REQUEST),
    invalid_hash ("The sql password hash is invalid" BAD_REQUEST),
    permissionless_entry ("One permission entry doesn't have privileges" BAD_REQUEST),
});
define_path_variable!(database_id "The database id");

#[service(path = "/database", tag = "database")]
pub trait DatabaseService {
    /// Creates a new database
    #[route("", method = PUT)]
    #[error(database(invalid_identifier) sql(failure [sql:*]))]
    #[example("examples/database.create.json")]
    async fn create_database(&self, name_suffix: String) -> Result<u32, _>;

    /// Updates an existing database
    #[route("/{database_id}", method = PATCH)]
    #[error(database(not_found invalid_identifier invalid_hash permissionless_entry) sql(failure [sql:*]))]
    #[example("examples/database.update.json")]
    async fn update_database(
        &self,
        database_id: u32,
        users: HashMap<String, DatabaseUserPayload>,
        passwordless: HashMap<u32, DatabaseUserPermissions>,
    ) -> Result<(), _>;

    /// Updates an existing database
    #[route("/{database_id}", method = DELETE)]
    #[error(database(not_found) sql(failure [sql:*]))]
    async fn delete_database(&self, database_id: u32) -> Result<(), _>;
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash, utoipa::ToSchema)]
pub struct DatabaseUserPayload {
    pub password_hash: String,
    pub permissions: DatabaseUserPermissions,
}

bitflags! {
    #[derive(Debug, Eq, PartialEq, Hash, Clone, Copy)]
    pub struct DatabaseUserPermissions: i8 {
        const SELECT = 1 << 0;
        const INSERT = 1 << 1;
        const UPDATE = 1 << 2;
        const DELETE = 1 << 3;
        const CREATE = 1 << 4;
        const DROP = 1 << 5;
        const INDEX = 1 << 6;
    }
}

impl PartialSchema for DatabaseUserPermissions {
    fn schema() -> RefOr<Schema> {
        i32::schema()
    }
}

impl ToSchema for DatabaseUserPermissions {
    fn name() -> Cow<'static, str> {
        Cow::Borrowed("DatabasePermissions")
    }
}

impl Serialize for DatabaseUserPermissions {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i8(self.bits())
    }
}

impl<'de> Deserialize<'de> for DatabaseUserPermissions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        i8::deserialize(deserializer).map(Self::from_bits_truncate)
    }
}
