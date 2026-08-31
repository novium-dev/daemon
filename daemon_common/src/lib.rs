// SPDX-License-Identifier: AGPL-3.0-only
#![forbid(unsafe_code)]

/// Fingerprint of the service definitions this crate was built from.
///
/// The daemon reports it so the backend can tell a node built against a
/// different API surface from one that merely sits on a different commit.
pub const API_HASH: &str = env!("API_HASH");

pub mod files;
#[cfg(feature = "service")]
pub mod integrity;
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
