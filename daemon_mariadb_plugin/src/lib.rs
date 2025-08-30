/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
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
