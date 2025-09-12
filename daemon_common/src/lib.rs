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
#![forbid(unsafe_code)]

pub mod files;
#[cfg(feature = "keys")]
pub mod key;
#[cfg(feature = "sdk")]
pub mod sdk;

#[cfg(feature = "backend")]
pub mod node;
#[cfg(feature = "service")]
pub mod service;
#[cfg(feature = "ssh")]
pub mod ssh;
#[cfg(feature = "service")]
pub mod user;
pub mod utils;

include!("../../config.rs");
