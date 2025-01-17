use crate::common::payload::database::{DatabaseModifyPayload, DatabaseUserPayload};
use crate::database::FetchedDatabase;
use daemon_common::payload::database::DatabasePermissions;

pub(crate) trait FromFetchedDatabaseExtension {
    fn from_fetched_database(value: FetchedDatabase) -> (String, Self);
}

impl FromFetchedDatabaseExtension for DatabaseModifyPayload {
    fn from_fetched_database(value: FetchedDatabase) -> (String, Self) {
        (
            value.name_suffix,
            DatabaseModifyPayload {
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
                                permissions: DatabasePermissions::from_bits_truncate(
                                    user.permissions,
                                ),
                                password_hash: user.password_hash,
                            },
                        )
                    })
                    .collect(),
            },
        )
    }
}
