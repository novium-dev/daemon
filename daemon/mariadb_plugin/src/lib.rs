#![warn(clippy::string_to_string)]

mod constants;
mod r#macro;
#[cfg(target_os = "linux")]
mod plugin;
mod types;
#[cfg(target_os = "linux")]
mod wrapper;

include!("../../../config.rs");
