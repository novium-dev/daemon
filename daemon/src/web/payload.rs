use crate::schema::FetchedDatabase;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize)]
pub(crate) struct ServerPayload {
    pub(crate) resource: Option<ServerResourcePayload>,
    pub(crate) build: Option<ServerBuildPayload>,

    pub(crate) ports: Option<Vec<ServerPortEntryPayload>>,
}

#[derive(Deserialize)]
pub(crate) struct ServerResourcePayload {
    pub(crate) cpu: Option<f32>,
    pub(crate) ram: Option<u32>,
    pub(crate) swap: Option<u32>,
    pub(crate) disk: Option<u64>,
}

#[derive(Deserialize)]
pub(crate) struct ServerBuildPayload {
    pub(crate) identifier: String,
    pub(crate) dockerfile: String,
    pub(crate) build_args: HashMap<String, String>,

    pub(crate) cap_add: Vec<String>,
    pub(crate) cap_drop: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct ServerPortEntryPayload {
    pub(crate) internal_port: u16,
    pub(crate) external_port: u16,
    pub(crate) external_ip: String,
    pub(crate) protocol: String,
}

#[derive(Serialize)]
pub(crate) struct IdResponse {
    pub(crate) id: u32,
}

#[derive(Deserialize, Eq, PartialEq)]
pub(crate) struct DatabasePayload {
    pub(crate) name_suffix: String,
    pub(crate) passwordless: HashMap<u32, i32>,
    pub(crate) users: HashMap<String, DatabaseUserPayload>,
}

impl From<FetchedDatabase> for DatabasePayload {
    fn from(value: FetchedDatabase) -> Self {
        DatabasePayload {
            name_suffix: value.name_suffix,
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

#[derive(Deserialize, Eq, PartialEq, Hash)]
pub(crate) struct DatabaseUserPayload {
    pub(crate) password_hash: String,
    pub(crate) permissions: i32,
    pub(crate) restricted: Option<Vec<u32>>,
}
