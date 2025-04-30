#![warn(clippy::string_to_string)]
#![forbid(unsafe_code)]

pub mod payload;

pub mod files;
pub mod server;
pub mod service;
pub mod ssh;
pub mod user;
pub mod utils;

pub use daemon_framework::FileResponse;
pub use daemon_framework::WebsocketResponse;
pub use daemon_framework::framework_router;
pub use daemon_framework::framework_router_user;

include!("../../../config.rs");
