// SPDX-License-Identifier: AGPL-3.0-only
use crate::service::backup::{
    BackupService, BackupServiceClientImpl, BackupServiceURL, BackupServiceUserKeyExtension,
};
use crate::service::database::{DatabaseService, DatabaseServiceClientImpl, DatabaseServiceURL};
use crate::service::error_storage_quota_reached::FromStorageQuotaReached;
use crate::service::files::{
    FilesService, FilesServiceClientImpl, FilesServiceURL, FilesServiceUserKeyExtension,
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
use uuid::Uuid;
pub(crate) use zelus::error::{define_error, error};
pub(crate) use zelus::service;
use zelus::{define_header_variable, define_path_variable};

pub use zelus::framework_router;
pub use zelus::responses::{FileResponse, WebsocketResponse};
pub use zelus::sdk::ZelusClientImpl;
pub use zelus::types::{DataStream, JsonVec};

pub mod backup;
pub mod database;
pub mod files;
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
        Self::new(io::ErrorKind::QuotaExceeded, "Quota reached")
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

pub trait DaemonServices:
    ServerService
    + BackupService
    + FilesService
    + NetworkService
    + ResourceService
    + DatabaseService
    + ScheduleService
    + SetupService
{
    fn id(&self) -> Uuid;
}
pub trait DaemonServicesClientImpl:
    ServerServiceClientImpl
    + BackupServiceClientImpl
    + FilesServiceClientImpl
    + NetworkServiceClientImpl
    + ResourceServiceClientImpl
    + DatabaseServiceClientImpl
    + ScheduleServiceClientImpl
    + SetupServiceClientImpl
{
}

pub trait DaemonServicesURL:
    DaemonServices
    + ServerServiceURL
    + BackupServiceURL
    + FilesServiceURL
    + NetworkServiceURL
    + ResourceServiceURL
    + DatabaseServiceURL
    + ScheduleServiceURL
    + SetupServiceURL
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
