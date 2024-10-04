use crate::schema::FetchedDatabase;
use crate::common::payload::database::{DatabasePayload, DatabaseUserPayload};

pub(crate) trait FromFetchedDatabaseExtension {
    fn from_fetched_database(value: FetchedDatabase) -> Self;
}

impl FromFetchedDatabaseExtension for DatabasePayload {
    fn from_fetched_database(value: FetchedDatabase) -> Self {
        DatabasePayload {
            name_suffix: Some(value.name_suffix),
            passwordless: value.passwordless,
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
                            permissions: user.permissions,
                            password_hash: user.password_hash,
                            restricted,
                        },
                    )
                })
                .collect(),
        }
    }
}
