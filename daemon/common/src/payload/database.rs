#[cfg(target_arch = "wasm32")]
use super::*;

#[cfg(not(target_arch = "wasm32"))]
use bitflags::bitflags;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
#[cfg(target_arch = "wasm32")]
use tsify::declare;

#[derive(Serialize, Deserialize, Eq, PartialEq)]
pub struct DatabasePayload {
    pub name_suffix: Option<String>,
    pub passwordless: HashMap<u32, DatabasePermissions>,
    pub users: HashMap<String, DatabaseUserPayload>,
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Hash)]
#[cfg_attr(target_arch = "wasm32", derive(Tsify))]
#[cfg_attr(target_arch = "wasm32", tsify(into_wasm_abi, from_wasm_abi))]
pub struct DatabaseUserPayload {
    pub password_hash: String,
    pub permissions: DatabasePermissions,
    pub restricted: Option<Vec<u32>>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(target_arch = "wasm32", derive(Tsify))]
#[cfg_attr(target_arch = "wasm32", tsify(into_wasm_abi, from_wasm_abi))]
#[serde(transparent)]
pub struct DatabaseUsersPayload(pub HashMap<String, DatabaseUserPayload>);

impl From<DatabaseUsersPayload> for HashMap<String, DatabaseUserPayload> {
    fn from(value: DatabaseUsersPayload) -> Self {
        value.0
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
pub struct DatabasePasswordlessPayload(pub HashMap<u32, DatabasePermissions>);

#[cfg(not(target_arch = "wasm32"))]
bitflags! {
    #[derive(Eq, PartialEq, Hash, Clone, Copy)]
    pub struct DatabasePermissions: i32 {
        const SELECT = 1 << 0;
        const INSERT = 1 << 1;
        const UPDATE = 1 << 2;
        const DELETE = 1 << 3;
        const CREATE = 1 << 4;
        const DROP = 1 << 5;
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Serialize for DatabasePermissions {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i32(self.bits())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<'de> Deserialize<'de> for DatabasePermissions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        i32::deserialize(deserializer).map(DatabasePermissions::from_bits_truncate)
    }
}

#[cfg(target_arch = "wasm32")]
#[declare]
pub type DatabasePermissions = i32;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(js_name = DatabasePermissionBits)]
pub enum WasmDatabasePermissions {
    SELECT = 1,
    INSERT = 2,
    UPDATE = 4,
    DELETE = 8,
    CREATE = 16,
    DROP = 32,
}

impl From<DatabasePasswordlessPayload> for HashMap<u32, DatabasePermissions> {
    fn from(value: DatabasePasswordlessPayload) -> Self {
        value.0
    }
}

impl From<HashMap<u32, DatabasePermissions>> for DatabasePasswordlessPayload {
    fn from(value: HashMap<u32, DatabasePermissions>) -> Self {
        DatabasePasswordlessPayload(value)
    }
}
