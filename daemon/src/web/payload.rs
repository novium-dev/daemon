use crate::common::payload::database::{DatabasePayload, DatabaseUserPayload};
use crate::schema::FetchedDatabase;
use daemon_common::payload::database::DatabasePermissions;

pub(crate) trait FromFetchedDatabaseExtension {
    fn from_fetched_database(value: FetchedDatabase) -> Self;
}

impl FromFetchedDatabaseExtension for DatabasePayload {
    fn from_fetched_database(value: FetchedDatabase) -> Self {
        DatabasePayload {
            name_suffix: Some(value.name_suffix),
            passwordless: value
                .passwordless
                .into_iter()
                .map(|(k, v)| (k, DatabasePermissions::from_bits_truncate(v)))
                .collect(),
            users: value
                .users
                .into_iter()
                .map(|(name_suffix, user)| {
                    (
                        name_suffix,
                        DatabaseUserPayload {
                            permissions: DatabasePermissions::from_bits_truncate(user.permissions),
                            password_hash: user.password_hash,
                        },
                    )
                })
                .collect(),
        }
    }
}
