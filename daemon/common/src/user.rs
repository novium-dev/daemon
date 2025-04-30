use crate::user::error_auth_signature_expired::FromAuthSignatureExpired;
use crate::user::error_auth_signature_invalid::FromAuthSignatureInvalid;
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use daemon_framework::{define_error, define_query_variable, paste};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::error;

define_query_variable!(verify "The signature to ensure the request is authorized");
define_query_variable!(timestamp "The timestamp the request was authorized. This is needed for verification");

define_error!(auth {
    signature_invalid ("The provided signature is not valid" UNAUTHORIZED),
    signature_expired ("The provided signature is expired" UNAUTHORIZED),
});

#[derive(Debug, Copy, Clone)]
pub struct UserSignature(Signature);

macro_rules! signature_impl {
    (
        $id:ident,
        $name:literal,
        ($($arg:tt)*),
        $extra:stmt
    ) => {

        paste::paste! {

            pub fn [< new_ $id >](key: &SigningKey, timestamp: i64, node_identifier: &str, server_id: u32, $( $arg )*) -> Self {
                Self::new(key, timestamp, node_identifier, server_id, $name, $extra)
            }

            pub fn [< verify_ $id >]<E: FromAuthSignatureInvalid + FromAuthSignatureExpired>(&self, key: &VerifyingKey, timestamp: i64, node_identifier: &str, server_id: u32, $( $arg )*) -> Result<(), E> {
                self.verify(key, timestamp, node_identifier, server_id, $name, $extra)
            }

        }

    };
}

impl UserSignature {
    const THRESHOLD: i128 = 60_000;

    fn new(
        key: &SigningKey,
        timestamp: i64,
        node_identifier: &str,
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
        key: &VerifyingKey,
        timestamp: i64,
        node_identifier: &str,
        server_id: u32,
        name: &str,
        extra: Option<String>,
    ) -> Result<(), E> {
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
        .map_err(|_| <E as FromAuthSignatureInvalid>::from())?;
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .inspect_err(|err| error!("Time error: {:?}", err))
            .map_err(|_| <E as FromAuthSignatureExpired>::from())?;
        if (timestamp as i128 + Self::THRESHOLD) as u128 <= unix.as_millis() {
            return Err(<E as FromAuthSignatureExpired>::from());
        }
        Ok(())
    }

    signature_impl!(backup_download, "backup", (backup_storage: impl Into<String>, backup_id: i64), Some(format!("-{}-{}", backup_storage.into(), backup_id)));
    signature_impl!(backup_realtime_download, "backup_rt", (compression: impl Into<String>), Some(format!("-{}", compression.into())));
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
            .decode(<&str>::deserialize(deserializer)?)
            .map_err(D::Error::custom)?;
        Signature::from_slice(&decoded)
            .map_err(D::Error::custom)
            .map(UserSignature)
    }
}
