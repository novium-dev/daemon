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
#![allow(clippy::disallowed_methods)]
#![expect(
    clippy::future_not_send,
    reason = "impl Future, send is determined by T"
)]

pub use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, broadcast};

#[cfg(feature = "trace_sync")]
pub mod mutex;
pub mod rwlock;

#[cfg(feature = "trace_sync")]
pub type Mutex<T> = mutex::TraceableMutex<T>;
#[cfg(not(feature = "trace_sync"))]
pub type Mutex<T> = tokio::sync::Mutex<T>;

#[cfg(feature = "trace_sync")]
pub type RwLock<T> = rwlock::TraceableRwLock<T>;
#[cfg(not(feature = "trace_sync"))]
pub type RwLock<T> = tokio::sync::RwLock<T>;
