use crate::common::payload::database::{DatabasePayload, DatabaseUserPayload};
use crate::db::schema::FetchedDatabase;
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
                    let mut restricted = None;
                    if let Some(str) = user.restricted {
                        restricted = Some(if str.is_empty() {
                            Vec::new()
                        } else {
                            str.split(';')
                                .map(|str| str.parse::<u32>().unwrap())
                                .collect()
                        });
                    }
                    (
                        name_suffix,
                        DatabaseUserPayload {
                            permissions: DatabasePermissions::from_bits_truncate(user.permissions),
                            password_hash: user.password_hash,
                            restricted,
                        },
                    )
                })
                .collect(),
        }
    }
}
