#![allow(clippy::disallowed_methods)]

pub use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, broadcast};

#[cfg(feature = "trace_sync")]
pub(crate) mod mutex;
pub(crate) mod rwlock;

#[cfg(feature = "trace_sync")]
pub type Mutex<T> = mutex::TraceableMutex<T>;
#[cfg(not(feature = "trace_sync"))]
pub type Mutex<T> = tokio::sync::Mutex<T>;

#[cfg(feature = "trace_sync")]
pub type RwLock<T> = rwlock::TraceableRwLock<T>;
#[cfg(not(feature = "trace_sync"))]
pub type RwLock<T> = tokio::sync::RwLock<T>;
