use super::*;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Eq, PartialEq)]
pub struct DatabasePayload {
    pub name_suffix: Option<String>,
    pub passwordless: HashMap<u32, i32>,
    pub users: HashMap<String, DatabaseUserPayload>,
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Hash)]
#[cfg_attr(target_arch = "wasm32", derive(Tsify))]
#[cfg_attr(target_arch = "wasm32", tsify(into_wasm_abi, from_wasm_abi))]
pub struct DatabaseUserPayload {
    pub password_hash: String,
    pub permissions: i32,
    pub restricted: Option<Vec<u32>>,
}


#[derive(Serialize, Deserialize)]
#[cfg_attr(target_arch = "wasm32", derive(Tsify))]
#[cfg_attr(target_arch = "wasm32", tsify(into_wasm_abi, from_wasm_abi))]
#[serde(transparent)]
pub struct DatabaseUsersPayload(pub HashMap<String, DatabaseUserPayload>);

impl Into<HashMap<String, DatabaseUserPayload>> for DatabaseUsersPayload {
    fn into(self) -> HashMap<String, DatabaseUserPayload> {
        self.0
    }
}

impl From<HashMap<String, DatabaseUserPayload>> for DatabaseUsersPayload {
    fn from(value: HashMap<String, DatabaseUserPayload>) -> Self {
        DatabaseUsersPayload(value)
    }
}


#[derive(Serialize, Deserialize)]
#[cfg_attr(target_arch = "wasm32", derive(Tsify))]
#[cfg_attr(target_arch = "wasm32", tsify(into_wasm_abi, from_wasm_abi))]
#[serde(transparent)]
pub struct DatabasePasswordlessPayload(pub HashMap<u32, i32>);

impl Into<HashMap<u32, i32>> for DatabasePasswordlessPayload {
    fn into(self) -> HashMap<u32, i32> {
        self.0
    }
}

impl From<HashMap<u32, i32>> for DatabasePasswordlessPayload {
    fn from(value: HashMap<u32, i32>) -> Self {
        DatabasePasswordlessPayload(value)
    }
}
