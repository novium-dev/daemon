// SPDX-License-Identifier: AGPL-3.0-only
// Credits go to https://github.com/Drako/mysql-plugin-api for finding out, how mariadb plugins work in rust

// The plugin relies on Linux-only APIs (e.g. SO_PEERCRED) and is never built on macOS
#![cfg(target_os = "linux")]
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
mod plugin;
mod types;
mod wrapper;

include!("../../config.rs");
