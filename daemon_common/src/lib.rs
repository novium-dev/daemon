// SPDX-License-Identifier: AGPL-3.0-only
#![forbid(unsafe_code)]

#[cfg(feature = "keys")]
pub mod auth;
pub mod files;
#[cfg(feature = "keys")]
pub mod key;
#[cfg(feature = "sdk")]
pub mod sdk;

#[cfg(any(feature = "backend", feature = "service"))]
pub mod node;
#[cfg(feature = "service")]
pub mod service;
#[cfg(feature = "ssh")]
pub mod ssh;
#[cfg(feature = "service")]
pub mod task;
#[cfg(feature = "service")]
pub mod user;
pub mod utils;

include!("../../config.rs");
