use crate::service::server::ServerService;
pub(crate) use daemon_framework::{define_error, service};
use daemon_framework::{define_header_variable, define_path_variable};

pub mod server;

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
        "The server storage does not exist" BAD_REQUEST
    ),
});

define_error!(sql {
    failure (
        "Database error occurred" INTERNAL_SERVER_ERROR
    )
});

define_path_variable!(server_id "The server id");
define_header_variable!(auth "The authentication token");

pub trait DaemonServices: ServerService {}
