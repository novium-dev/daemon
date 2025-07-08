// Credits go to https://github.com/Drako/mysql-plugin-api for finding out, how mariadb plugins work in rust

#![expect(clippy::print_stdout, reason = "no logger available here")]
#![expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "most of them are defined in the mariadb headers"
)]
#![expect(
    clippy::struct_field_names,
    reason = "most of them are defined in the mariadb headers"
)]

mod constants;
mod r#macro;
#[cfg(target_os = "linux")]
mod plugin;
mod types;
#[cfg(target_os = "linux")]
mod wrapper;

include!("../../../config.rs");
