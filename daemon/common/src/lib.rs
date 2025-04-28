#![warn(clippy::string_to_string)]
#![forbid(unsafe_code)]

pub mod payload;

pub mod files;
pub mod server;
pub mod service;
pub mod ssh;
pub mod utils;

pub use daemon_framework::framework_router;
pub use daemon_framework::framework_router_user;

include!("../../../config.rs");
