#![warn(clippy::string_to_string)]
#![forbid(unsafe_code)]

pub mod payload;

pub mod files;
pub mod server;
pub mod ssh;
pub mod utils;

include!("../../../config.rs");
