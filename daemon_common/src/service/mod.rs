// SPDX-License-Identifier: AGPL-3.0-only
use crate::service::backup::{
    BackupService, BackupServiceClientImpl, BackupServiceURL, BackupServiceUserKeyExtension,
};
#[cfg(feature = "databases")]
use crate::service::database::{DatabaseService, DatabaseServiceClientImpl, DatabaseServiceURL};
use crate::service::error_storage_quota_reached::FromStorageQuotaReached;
use crate::service::files::{
    FilesService, FilesServiceClientImpl, FilesServiceURL, FilesServiceUserKeyExtension,
};
use crate::service::integrity::{
    IntegrityService, IntegrityServiceClientImpl, IntegrityServiceURL,
};
use crate::service::network::{NetworkService, NetworkServiceClientImpl, NetworkServiceURL};
use crate::service::resource::{ResourceService, ResourceServiceClientImpl, ResourceServiceURL};
use crate::service::schedule::{ScheduleService, ScheduleServiceClientImpl, ScheduleServiceURL};
use crate::service::server::{
    ServerService, ServerServiceClientImpl, ServerServiceURL, ServerServiceUserKeyExtension,
};
use crate::service::setup::{SetupService, SetupServiceClientImpl, SetupServiceURL};
use crate::user::UserKeyProvider;
use std::io;
use std::io::ErrorKind;
use uuid::Uuid;
pub(crate) use zelus::error::{define_error, error};
pub(crate) use zelus::service;
use zelus::{define_header_variable, define_path_variable};

use crate::service::group::{GroupService, GroupServiceClientImpl, GroupServiceURL};
pub use zelus::framework_router;
pub use zelus::responses::{FileResponse, WebsocketResponse};
pub use zelus::sdk::ZelusClientImpl;
pub use zelus::types::{DataStream, JsonVec};

pub mod backup;
#[cfg(feature = "databases")]
pub mod database;
pub mod files;
pub mod group;
pub mod integrity;
pub mod network;
pub mod resource;
pub mod schedule;
pub mod server;
pub mod setup;

define_error!(auth {
    invalid (
        "Authentication is invalid" UNAUTHORIZED
    ),
});
define_error!(generic {});

define_error!(server {
    not_found ("The server does not exist" NOT_FOUND),
    mutex_locked ("The server is locked (currently in use)" SERVICE_UNAVAILABLE),
});

define_error!(docker {
    failure ("Docker error occurred" INTERNAL_SERVER_ERROR),
});

define_error!(storage {
    not_found (
        "The storage does not exist" NOT_FOUND
    ),
    location_not_found (
        "The storage location does not exist" BAD_REQUEST
    ),
    failure (
        "The server storage failed" INTERNAL_SERVER_ERROR
    ),
    mutex_locked ("The server storage is locked (currently in use)" SERVICE_UNAVAILABLE),
    quota_reached ("The server storage disk limit has been reached" FORBIDDEN),
    patch_failed ("The patch did not succeed" FORBIDDEN )
});

impl FromStorageQuotaReached for io::Error {
    fn from() -> Self
    where
        Self: Sized,
    {
        Self::new(ErrorKind::QuotaExceeded, "Quota reached")
    }
}

define_error!(sql {
    failure (
        "Database error occurred" INTERNAL_SERVER_ERROR
    )
});

define_error!(compression {
    unsupported ( "The specified compression is not supported" BAD_REQUEST )
});
define_error!(archive {
    corrupted ( "The archive is corrupted" FORBIDDEN )
});

define_error!(http_request {
    failed ( "Unable to send http request" FORBIDDEN ),
    not_success ( "Http request returned non-success response" FORBIDDEN ),
    read_failed ( "Reading from http request failed" FORBIDDEN ),
});

define_path_variable!(server_id "The server id");
define_header_variable!(auth "The authentication token");

error!(BlankError, auth::invalid);

/// Builds the `OpenAPI` document describing every route the daemon serves.
///
/// Both the daemon and the backend can call this: the routes come from the
/// service definitions in this crate, and the instance only backs the handler
/// closures, which the document does not describe.
#[must_use]
pub fn api_document<T>(instance: &T) -> utoipa::openapi::OpenApi
where
    T: DaemonServices + Clone + Send + Sync + Sized + 'static,
{
    use utoipa::OpenApi as _;

    #[derive(utoipa::OpenApi)]
    #[openapi()]
    struct ApiDoc;

    #[cfg(feature = "databases")]
    let (with_auth, without_auth) = framework_router!(T instance (with_auth,without_auth,) {
        ServerService, BackupService, FilesService, NetworkService,
        ResourceService, DatabaseService, ScheduleService, SetupService,
        IntegrityService, GroupService
    });
    #[cfg(not(feature = "databases"))]
    let (with_auth, without_auth) = framework_router!(T instance (with_auth,without_auth,) {
        ServerService, BackupService, FilesService, NetworkService,
        ResourceService, ScheduleService, SetupService,
        IntegrityService, GroupService
    });

    let (_router, api) = utoipa_axum::router::OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(with_auth.into_openapi())
        .merge(without_auth.into_openapi())
        .split_for_parts();

    api
}

/// Fingerprint of the daemon's `OpenAPI` document, so the backend can tell a node
/// serving a different API surface from one that merely sits on a different
/// commit.
#[must_use]
pub fn api_hash<T>(instance: &T) -> String
where
    T: DaemonServices + Clone + Send + Sync + Sized + 'static,
{
    use core::fmt::Write as _;
    use sha2::Digest as _;

    let document = serde_json::to_string(&api_document(instance)).unwrap_or_default();

    let mut out = String::new();
    for byte in sha2::Sha256::digest(document.as_bytes()) {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Services that only exist with the `databases` feature.
#[cfg(feature = "databases")]
pub trait DaemonOptionalServices: DatabaseService {}
#[cfg(feature = "databases")]
impl<T: DatabaseService> DaemonOptionalServices for T {}
#[cfg(not(feature = "databases"))]
pub trait DaemonOptionalServices {}
#[cfg(not(feature = "databases"))]
impl<T> DaemonOptionalServices for T {}

#[cfg(feature = "databases")]
pub trait DaemonOptionalServicesClientImpl: DatabaseServiceClientImpl {}
#[cfg(feature = "databases")]
impl<T: DatabaseServiceClientImpl> DaemonOptionalServicesClientImpl for T {}
#[cfg(not(feature = "databases"))]
pub trait DaemonOptionalServicesClientImpl {}
#[cfg(not(feature = "databases"))]
impl<T> DaemonOptionalServicesClientImpl for T {}

#[cfg(feature = "databases")]
pub trait DaemonOptionalServicesURL: DatabaseServiceURL {}
#[cfg(feature = "databases")]
impl<T: DatabaseServiceURL> DaemonOptionalServicesURL for T {}
#[cfg(not(feature = "databases"))]
pub trait DaemonOptionalServicesURL {}
#[cfg(not(feature = "databases"))]
impl<T> DaemonOptionalServicesURL for T {}

pub trait DaemonServices:
    ServerService
    + BackupService
    + FilesService
    + NetworkService
    + ResourceService
    + DaemonOptionalServices
    + ScheduleService
    + SetupService
    + IntegrityService
    + GroupService
{
    fn id(&self) -> Uuid;
}
pub trait DaemonServicesClientImpl:
    ServerServiceClientImpl
    + BackupServiceClientImpl
    + FilesServiceClientImpl
    + NetworkServiceClientImpl
    + ResourceServiceClientImpl
    + DaemonOptionalServicesClientImpl
    + ScheduleServiceClientImpl
    + SetupServiceClientImpl
    + IntegrityServiceClientImpl
    + GroupServiceClientImpl
{
}

pub trait DaemonServicesURL:
    DaemonServices
    + ServerServiceURL
    + BackupServiceURL
    + FilesServiceURL
    + NetworkServiceURL
    + ResourceServiceURL
    + DaemonOptionalServicesURL
    + ScheduleServiceURL
    + SetupServiceURL
    + IntegrityServiceURL
    + GroupServiceURL
{
}

pub trait DaemonServicesURLWithUserKey:
    DaemonServicesURL
    + ServerServiceUserKeyExtension
    + BackupServiceUserKeyExtension
    + FilesServiceUserKeyExtension
    + UserKeyProvider
{
}

#[cfg(all(test, feature = "sdk"))]
mod tests {
    use super::api_hash;
    use crate::key::rcgen::KeyPair;
    use crate::key::{BackendKey, DaemonRootKey};
    use crate::sdk::DaemonSDK;
    use ed25519_dalek::SigningKey;
    use std::sync::Arc;
    use url::Url;
    use uuid::Uuid;

    fn keys() -> (Arc<DaemonRootKey>, Arc<BackendKey>) {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let root_keypair = KeyPair::generate().expect("root keypair");
        let root = DaemonRootKey::new(&root_keypair, None).expect("root key");
        let backend_keypair = KeyPair::generate().expect("backend keypair");
        let backend = root
            .new_backend_key(&root_keypair, &backend_keypair)
            .expect("backend key");

        (Arc::new(root), Arc::new(backend))
    }

    #[test]
    fn api_hash_does_not_depend_on_the_instance() {
        let (root, backend) = keys();

        let plain = DaemonSDK::builder()
            .with_identifier(Uuid::nil())
            .with_root_key(Arc::clone(&root))
            .with_backend_key(Arc::clone(&backend))
            .with_url(Url::parse("https://one.invalid").expect("url"))
            .build();

        let with_user_key = DaemonSDK::builder()
            .with_identifier(Uuid::max())
            .with_root_key(root)
            .with_backend_key(backend)
            .with_url(Url::parse("https://two.invalid").expect("url"))
            .with_user_key(SigningKey::from_bytes(&[7u8; 32]))
            .build();

        let first = api_hash(&plain);
        let second = api_hash(&with_user_key);

        assert_eq!(
            first, second,
            "the hash must describe the api, not the client it was built from"
        );
        assert_eq!(first.len(), 64, "sha256 renders as 64 hex characters");
        println!("API_HASH={first}");
    }
}
