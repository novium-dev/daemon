#![allow(clippy::disallowed_methods)]
#![expect(clippy::future_not_send, reason = "impl Future, send is determined by T")]

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
