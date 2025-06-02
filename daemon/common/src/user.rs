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
use uuid::Uuid;

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

            pub fn [< new_ $id >](key: &SigningKey, timestamp: i64, node_identifier: &uuid::Uuid, server_id: u32, $( $arg )*) -> Self {
                Self::new(key, timestamp, node_identifier, server_id, $name, $extra)
            }

            pub fn [< verify_ $id >]<E: FromAuthSignatureInvalid + FromAuthSignatureExpired>(&self, key: &Option<VerifyingKey>, timestamp: i64, node_identifier: &uuid::Uuid, server_id: u32, $( $arg )*) -> Result<(), E> {
                self.verify(key, timestamp, node_identifier, server_id, $name, $extra)
            }

        }

    };
}

#[cfg(feature = "ts")]
macro_rules! signature_ts_impl {
    (
        $visitor:ident,
        $id:literal,
        $name:literal,
        ($args: literal),
        $extra:literal
    ) => {
        $visitor.out.push_str(&format!(r#"
  static {}(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number{}{}): UserSignature {{
    return UserSignature.new(privateKey, timestamp, nodeIdentifier, serverId, {}, {});
  }}
"#, daemon_framework::ts::to_camel(&format!("new_{}", $id)), if ($args).is_empty() { "" } else { ", " }, $args, serde_json::to_string($name).unwrap(), $extra));

        $visitor.out.push_str(&format!(r#"
  static async {}(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number{}{}): Promise<UserSignature> {{
    return await UserSignature.newAsync(privateKey, timestamp, nodeIdentifier, serverId, {}, {});
  }}
"#, daemon_framework::ts::to_camel(&format!("new_{}_async", $id)), if ($args).is_empty() { "" } else { ", " }, $args, serde_json::to_string($name).unwrap(), $extra));


        $visitor.out.push_str(&format!(r#"
  {}(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number{}{}): boolean {{
    return this.verify(publicKey, timestamp, nodeIdentifier, serverId, {}, {});
  }}
"#, daemon_framework::ts::to_camel(&format!("verify_{}", $id)), if ($args).is_empty() { "" } else { ", " }, $args, serde_json::to_string($name).unwrap(), $extra));

        $visitor.out.push_str(&format!(r#"
  async {}(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number{}{}): Promise<boolean> {{
    return await this.verifyAsync(publicKey, timestamp, nodeIdentifier, serverId, {}, {});
  }}
"#, daemon_framework::ts::to_camel(&format!("verify_{}_async", $id)), if ($args).is_empty() { "" } else { ", " }, $args, serde_json::to_string($name).unwrap(), $extra));
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
        key: &Option<VerifyingKey>,
        timestamp: i64,
        node_identifier: &Uuid,
        server_id: u32,
        name: &str,
        extra: Option<String>,
    ) -> Result<(), E> {
        let key = key
            .as_ref()
            .ok_or(<E as FromAuthSignatureInvalid>::from())?;
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

    #[cfg(feature = "ts")]
    pub fn visit_types(visitor: &mut daemon_framework::ts::TSTypeVisitor) {
        visitor.out.push_str(r#"
export class UserSignature {
  readonly signature: Uint8Array;

  constructor(signature: Uint8Array) {
    this.signature = signature;
  }

  static new(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, name: string, extra: string | null) {
    return new UserSignature(ed.sign(UserSignature.content(timestamp, nodeIdentifier, serverId, name, extra), privateKey));
  }

  static async newAsync(privateKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, name: string, extra: string | null): Promise<UserSignature> {
    return new UserSignature(await ed.signAsync(UserSignature.content(timestamp, nodeIdentifier, serverId, name, extra), privateKey));
  }

  verify(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, name: string, extra: string | null): boolean {
    return ed.verify(this.signature, UserSignature.content(timestamp, nodeIdentifier, serverId, name, extra), publicKey);
  }

  async verifyAsync(publicKey: Uint8Array | string, timestamp: bigint, nodeIdentifier: string, serverId: number, name: string, extra: string | null): Promise<boolean> {
    return await ed.verifyAsync(this.signature, UserSignature.content(timestamp, nodeIdentifier, serverId, name, extra), publicKey);
  }

  private static content(timestamp: bigint, node_identifier: string, server_id: number, name: string, extra: string | null): string {
    return `${name}-${node_identifier}-${server_id}-${timestamp}${extra === null ? "" : extra}`;
  }

  toString(): string {
    return Buffer.from(this.signature).toString('base64');
  }
"#);

        signature_ts_impl!(
            visitor,
            "server_backup_download",
            "backup",
            ("backupStorage: string, backupId: bigint"),
            r#"`-${backupStorage}-${backupId}`"#
        );
        signature_ts_impl!(
            visitor,
            "server_realtime_download",
            "backup_rt",
            ("compression: string"),
            r#"`-${compression}`"#
        );
        signature_ts_impl!(
            visitor,
            "terminal",
            "terminal",
            ("write: boolean"),
            r#"`-${write}`"#
        );
        signature_ts_impl!(visitor, "stats", "stats", (""), r#"null"#);
        signature_ts_impl!(
            visitor,
            "exec",
            "exec",
            ("command: Array<string>"),
            r#"`-${command.join(";")}`"#
        );
        signature_ts_impl!(
            visitor,
            "server_file_download",
            "server_file_download",
            ("path: string"),
            r#"`-${path}`"#
        );
        signature_ts_impl!(
            visitor,
            "server_file_upload",
            "server_file_upload",
            ("path: string"),
            r#"`-${path}`"#
        );

        visitor.out.push_str("}\n");
    }

    signature_impl!(server_backup_download, "backup", (backup_storage: impl Into<String>, backup_id: i64), Some(format!("-{}-{}", backup_storage.into(), backup_id)));
    signature_impl!(server_realtime_download, "backup_rt", (compression: impl Into<String>), Some(format!("-{}", compression.into())));
    signature_impl!(terminal, "terminal", (write: bool), Some(format!("-{}", write)));
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
            .decode(<&str>::deserialize(deserializer)?)
            .map_err(D::Error::custom)?;
        Signature::from_slice(&decoded)
            .map_err(D::Error::custom)
            .map(UserSignature)
    }
}

#[cfg(feature = "ts")]
impl daemon_framework::ts_rs::TS for UserSignature {
    type WithoutGenerics = <String as daemon_framework::ts_rs::TS>::WithoutGenerics;
    const DOCS: Option<&'static str> = String::DOCS;

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
        "UserSignature".to_string()
    }

    fn inline() -> String {
        Self::name()
    }

    fn inline_flattened() -> String {
        Self::name()
    }
}

pub trait UserKeyProvider {
    fn node_id(&self) -> &Uuid;

    fn key(&self) -> &SigningKey;
}
