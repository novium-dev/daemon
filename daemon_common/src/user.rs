// SPDX-License-Identifier: AGPL-3.0-only
use crate::user::error_auth_signature_expired::FromAuthSignatureExpired;
use crate::user::error_auth_signature_invalid::FromAuthSignatureInvalid;
use base64::Engine as _;
use base64::prelude::BASE64_STANDARD;
use chrono::Utc;
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use utoipa::openapi::schema::SchemaType;
use utoipa::openapi::{Object, Ref, RefOr, Schema, Type};
use utoipa::{PartialSchema, ToSchema};
use uuid::Uuid;
use zelus::error::define_error;
use zelus::{define_query_variable, paste};

define_query_variable!(verify "The signature to ensure the request is authorized");
define_query_variable!(timestamp "The timestamp the request was authorized. This is needed for verification");

define_error!(auth {
    signature_invalid ("The provided signature is not valid" UNAUTHORIZED),
    signature_expired ("The provided signature is expired" UNAUTHORIZED),
});

/// An already parsed signature.
#[derive(Debug, Copy, Clone)]
#[must_use]
pub struct UserSignature(Signature);

macro_rules! signature_impl {
    (
        $id:ident,
        $name:literal,
        ($($arg:tt)*),
        $extra:stmt
    ) => {

        paste! {

            pub fn [< new_ $id >](key: &SigningKey, timestamp: i64, node_identifier: &uuid::Uuid, server_id: u32, $( $arg )*) -> Self {
                Self::new(key, timestamp, node_identifier, server_id, $name, $extra)
            }

            pub fn [< verify_ $id >]<E: FromAuthSignatureInvalid + FromAuthSignatureExpired>(&self, key: Option<&VerifyingKey>, timestamp: i64, node_identifier: &uuid::Uuid, server_id: u32, $( $arg )*) -> Result<(), E> {
                self.verify(key, timestamp, node_identifier, server_id, $name, $extra)
            }

        }

    };
}

impl UserSignature {
    const THRESHOLD: i64 = 60_000;

    fn new(
        key: &SigningKey,
        timestamp: i64,
        node_identifier: &Uuid,
        server_id: u32,
        name: &str,
        extra: Option<String>,
    ) -> Self {
        Self(
            key.sign(
                format!(
                    "{}-{}-{}-{}{}",
                    name,
                    node_identifier,
                    server_id,
                    timestamp,
                    extra.unwrap_or_default()
                )
                .as_bytes(),
            ),
        )
    }

    fn verify<E: FromAuthSignatureInvalid + FromAuthSignatureExpired>(
        &self,
        key: Option<&VerifyingKey>,
        timestamp: i64,
        node_identifier: &Uuid,
        server_id: u32,
        name: &str,
        extra: Option<String>,
    ) -> Result<(), E> {
        tracing::info!(
            "Verifying Content: {name}, Server ID: {server_id}, Node: {node_identifier}, Timestamp: {timestamp}, Extra Content: {extra:?}"
        );
        let key = key
            .as_ref()
            .ok_or_else(<E as FromAuthSignatureInvalid>::from)?;
        key.verify(
            format!(
                "{}-{}-{}-{}{}",
                name,
                node_identifier,
                server_id,
                timestamp,
                extra.unwrap_or_default()
            )
            .as_bytes(),
            &self.0,
        )
        .inspect_err(|_err| tracing::info!("Signature is invalid!"))
        .map_err(|_err| <E as FromAuthSignatureInvalid>::from())?;
        tracing::info!("Signature is correct, checking validity...");
        let unix = Utc::now().timestamp();
        tracing::info!(
            "Provided timestamp: {timestamp}, Current timestamp: {unix}. Threshold: {}",
            Self::THRESHOLD
        );
        if timestamp.saturating_add(Self::THRESHOLD) <= unix {
            tracing::info!("Signature is expired");
            return Err(<E as FromAuthSignatureExpired>::from());
        }
        tracing::info!("Signature is valid");

        Ok(())
    }

    signature_impl!(server_backup_download, "backup", (backup_storage: impl Into<String>, backup_id: i64), Some(format!("-{}-{}", backup_storage.into(), backup_id)));
    signature_impl!(server_realtime_download, "backup_rt", (compression: impl Into<String>), Some(format!("-{}", compression.into())));
    signature_impl!(terminal, "terminal", (write: bool), Some(format!("-{write}")));
    signature_impl!(stats, "stats", (), None);
    signature_impl!(exec, "exec", (command: &[String]), Some(format!("-{}", command.join(";"))));
    signature_impl!(server_file_download, "server_file_download", (path: impl Into<String>), Some(format!("-{}", path.into())));
    signature_impl!(server_file_upload, "server_file_upload", (path: impl Into<String>), Some(format!("-{}", path.into())));
    signature_impl!(server_watch_files, "server_watch_files", (), None);
}

impl PartialSchema for UserSignature {
    fn schema() -> RefOr<Schema> {
        RefOr::Ref(Ref::from_schema_name(Self::name()))
    }
}

impl ToSchema for UserSignature {
    fn name() -> Cow<'static, str> {
        Cow::Borrowed("UserSignature")
    }

    fn schemas(schemas: &mut Vec<(String, RefOr<Schema>)>) {
        schemas.push((
            Self::name().to_string(),
            RefOr::T(Schema::Object(
                Object::builder()
                    .schema_type(SchemaType::Type(Type::String))
                    .min_length(Some(88))
                    .max_length(Some(88))
                    .pattern(Some("(?:[A-Za-z0-9+\\/]{86})=="))
                    .build(),
            )),
        ));
    }
}

impl Serialize for UserSignature {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        BASE64_STANDARD
            .encode(self.0.to_bytes())
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for UserSignature {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let decoded = BASE64_STANDARD
            .decode(<String>::deserialize(deserializer)?)
            .map_err(D::Error::custom)?;
        Signature::from_slice(&decoded)
            .map_err(D::Error::custom)
            .map(UserSignature)
    }
}

pub trait UserKeyProvider {
    fn node_id(&self) -> &Uuid;

    fn key(&self) -> &SigningKey;
}
