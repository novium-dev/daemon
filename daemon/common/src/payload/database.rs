use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Eq, PartialEq)]
pub struct DatabasePayload {
    pub name_suffix: String,
    pub passwordless: HashMap<u32, i32>,
    pub users: HashMap<String, DatabaseUserPayload>,
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Hash)]
pub struct DatabaseUserPayload {
    pub password_hash: String,
    pub permissions: i32,
    pub restricted: Option<Vec<u32>>,
}
