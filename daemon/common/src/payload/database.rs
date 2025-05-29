use bitflags::bitflags;
use daemon_framework::ts;
use serde::{Deserialize, Serialize};
use serde::{Deserializer, Serializer};
use std::borrow::Cow;
use utoipa::openapi::{RefOr, Schema};
use utoipa::{PartialSchema, ToSchema};

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, Hash, utoipa::ToSchema)]
#[ts]
pub struct DatabaseUserPayload {
    pub password_hash: String,
    #[ts(as = "i8")]
    pub permissions: DatabaseUserPermissions,
}

bitflags! {
    #[derive(Debug, Eq, PartialEq, Hash, Clone, Copy)]
    #[ts(as = "i8")]
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
#[cfg(feature = "ts")]
impl DatabaseUserPermissions {
    pub fn visit_types(visitor: &mut daemon_framework::ts::TSTypeVisitor) {
        visitor
            .out
            .push_str("namespace DatabaseUserPermissions {\n");
        for (name, value) in DatabaseUserPermissions::all().iter_names() {
            visitor.out.push_str("  const ");
            visitor.out.push_str(name);
            visitor.out.push_str(": ");
            visitor
                .out
                .push_str(&<Self as daemon_framework::ts_rs::TS>::inline());
            visitor.out.push_str(" = ");
            visitor.out.push_str(&value.bits().to_string());
            visitor.out.push(';');
            visitor.out.push('\n');
        }
        visitor.out.push_str("}\n");
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
        i8::deserialize(deserializer).map(DatabaseUserPermissions::from_bits_truncate)
    }
}
