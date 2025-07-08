/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
use crate::user::error_auth_signature_expired::FromAuthSignatureExpired;
use crate::user::error_auth_signature_invalid::FromAuthSignatureInvalid;
use base64::Engine as _;
use base64::prelude::BASE64_STANDARD;
use daemon_framework::{define_error, define_query_variable, paste};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::error;
use uuid::Uuid;

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

        paste::paste! {

            pub fn [< new_ $id >](key: &SigningKey, timestamp: i64, node_identifier: &uuid::Uuid, server_id: u32, $( $arg )*) -> Self {
                Self::new(key, timestamp, node_identifier, server_id, $name, $extra)
            }

            pub fn [< verify_ $id >]<E: FromAuthSignatureInvalid + FromAuthSignatureExpired>(&self, key: Option<&VerifyingKey>, timestamp: i64, node_identifier: &uuid::Uuid, server_id: u32, $( $arg )*) -> Result<(), E> {
                self.verify(key, timestamp, node_identifier, server_id, $name, $extra)
            }

        }

    };
}

#[cfg(feature = "ts")]
macro_rules! signature_ts_impl {
    (
        $replace:ident,
        $id:literal,
        $name:literal,
        ($args: literal),
        $extra:literal
    ) => {
        {
            use core::fmt::Write as _;
            let _ = write!(&mut $replace, r"
  static {}(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number{}{}): UserSignature {{
    return UserSignature.new(privateKey, timestamp, nodeIdentifier, serverId, {}, {});
  }}
", daemon_framework::ts::to_camel(&format!("new_{}", $id)), if ($args).is_empty() { "" } else { ", " }, $args, serde_json::to_string($name).unwrap(), $extra);

            let _ = write!(&mut $replace, r"
  static async {}(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number{}{}): Promise<UserSignature> {{
    return await UserSignature.newAsync(privateKey, timestamp, nodeIdentifier, serverId, {}, {});
  }}
", daemon_framework::ts::to_camel(&format!("new_{}_async", $id)), if ($args).is_empty() { "" } else { ", " }, $args, serde_json::to_string($name).unwrap(), $extra);


            let _ = write!(&mut $replace, r"
  {}(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number{}{}): boolean {{
    return this.verify(publicKey, timestamp, nodeIdentifier, serverId, {}, {});
  }}
", daemon_framework::ts::to_camel(&format!("verify_{}", $id)), if ($args).is_empty() { "" } else { ", " }, $args, serde_json::to_string($name).unwrap(), $extra);

            let _ = write!(&mut $replace, r"
  async {}(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number{}{}): Promise<boolean> {{
    return await this.verifyAsync(publicKey, timestamp, nodeIdentifier, serverId, {}, {});
  }}
", daemon_framework::ts::to_camel(&format!("verify_{}_async", $id)), if ($args).is_empty() { "" } else { ", " }, $args, serde_json::to_string($name).unwrap(), $extra);
        }
    };
}

impl UserSignature {
    const THRESHOLD: i128 = 60_000;

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
        .map_err(|_err| <E as FromAuthSignatureInvalid>::from())?;
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .inspect_err(|err| error!("Time error: {:?}", err))
            .map_err(|_err| <E as FromAuthSignatureExpired>::from())?;
        if i128::from(timestamp)
            .saturating_add(Self::THRESHOLD)
            .unsigned_abs()
            <= unix.as_millis()
        {
            return Err(<E as FromAuthSignatureExpired>::from());
        }
        Ok(())
    }

    #[cfg(feature = "ts")]
    #[expect(clippy::cognitive_complexity, reason = "isn't that complex")]
    pub fn visit_types(visitor: &mut daemon_framework::ts::TSTypeVisitor) {
        let mut replace = String::new();

        signature_ts_impl!(
            replace,
            "server_backup_download",
            "backup",
            ("backupStorage: string, backupId: bigint"),
            "`-${backupStorage}-${backupId}`"
        );
        signature_ts_impl!(
            replace,
            "server_realtime_download",
            "backup_rt",
            ("compression: string"),
            "`-${compression}`"
        );
        signature_ts_impl!(
            replace,
            "terminal",
            "terminal",
            ("write: boolean"),
            "`-${write}`"
        );
        signature_ts_impl!(replace, "stats", "stats", (""), "null");
        signature_ts_impl!(
            replace,
            "exec",
            "exec",
            ("command: Array<string>"),
            r#"`-${command.join(";")}`"#
        );
        signature_ts_impl!(
            replace,
            "server_file_download",
            "server_file_download",
            ("path: string"),
            "`-${path}`"
        );
        signature_ts_impl!(
            replace,
            "server_file_upload",
            "server_file_upload",
            ("path: string"),
            "`-${path}`"
        );

        visitor
            .out
            .push_str(&include_str!("sdk/ts/user.ts").replace("/** {user_methods} */", &replace));
    }

    signature_impl!(server_backup_download, "backup", (backup_storage: impl Into<String>, backup_id: i64), Some(format!("-{}-{}", backup_storage.into(), backup_id)));
    signature_impl!(server_realtime_download, "backup_rt", (compression: impl Into<String>), Some(format!("-{}", compression.into())));
    signature_impl!(terminal, "terminal", (write: bool), Some(format!("-{write}")));
    signature_impl!(stats, "stats", (), None);
    signature_impl!(exec, "exec", (command: &[String]), Some(format!("-{}", command.join(";"))));
    signature_impl!(server_file_download, "server_file_download", (path: impl Into<String>), Some(format!("-{}", path.into())));
    signature_impl!(server_file_upload, "server_file_upload", (path: impl Into<String>), Some(format!("-{}", path.into())));
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

#[cfg(feature = "ts")]
#[expect(clippy::panic, reason = "Type cannot be declared")]
impl daemon_framework::ts_rs::TS for UserSignature {
    type WithoutGenerics = Self;
    type OptionInnerType = Self;

    fn decl() -> String {
        panic!(
            "{} cannot be declared",
            <Self as daemon_framework::ts_rs::TS>::name()
        )
    }

    fn decl_concrete() -> String {
        Self::decl()
    }

    fn name() -> String {
        "UserSignature".to_owned()
    }

    fn inline() -> String {
        Self::name()
    }

    fn inline_flattened() -> String {
        Self::name()
    }

    fn docs() -> Option<String> {
        None
    }
}

pub trait UserKeyProvider {
    fn node_id(&self) -> &Uuid;

    fn key(&self) -> &SigningKey;
}
