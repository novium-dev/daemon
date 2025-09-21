// SPDX-License-Identifier: AGPL-3.0-only
use core::future::Future;
use core::panic::Location;
use std::sync::Arc;
use tokio::sync::{Mutex as TokioMutex, MutexGuard, OwnedMutexGuard, TryLockError};

pub type MutexTraceData = Option<(&'static str, u32, u32, chrono::DateTime<chrono::Utc>)>;
pub struct TraceableMutex<T: ?Sized>(Arc<TokioMutex<T>>, std::sync::Mutex<MutexTraceData>);

impl<T: ?Sized> TraceableMutex<T> {
    pub fn new(value: T) -> Self
    where
        T: Sized,
    {
        Self(
            Arc::new(tokio::sync::Mutex::new(value)),
            std::sync::Mutex::default(),
        )
    }

    pub fn last_trace(&self) -> MutexTraceData {
        self.1.lock().map(|data| *data).unwrap_or_default()
    }

    fn __trace(&self, location: &'static Location<'static>) {
        let file = location.file();
        let line = location.line();
        let column = location.column();
        let time = chrono::Utc::now();
        if let Ok(mut data) = self.1.lock() {
            *data = Some((file, line, column, time));
        }
    }

    #[track_caller]
    pub fn lock(&self) -> impl Future<Output = MutexGuard<'_, T>> {
        let location = Location::caller();
        async move {
            let guard = self.0.lock().await;
            self.__trace(location);
            guard
        }
    }

    #[track_caller]
    pub fn lock_owned(self: Arc<Self>) -> impl Future<Output = OwnedMutexGuard<T>> {
        let location = Location::caller();
        async move {
            let guard = Arc::clone(&self.0).lock_owned().await;
            self.__trace(location);
            guard
        }
    }

    #[track_caller]
    pub fn blocking_lock(&self) -> MutexGuard<'_, T> {
        let guard = self.0.blocking_lock();
        self.__trace(Location::caller());
        guard
    }

    #[track_caller]
    pub fn blocking_lock_owned(self: Arc<Self>) -> OwnedMutexGuard<T> {
        let guard = Arc::clone(&self.0).blocking_lock_owned();
        self.__trace(Location::caller());
        guard
    }

    #[track_caller]
    pub fn try_lock(&self) -> Result<MutexGuard<'_, T>, TryLockError> {
        let guard = self.0.try_lock();
        if guard.is_ok() {
            self.__trace(Location::caller());
        }
        guard
    }

    pub fn try_lock_hidden(&self) -> Result<MutexGuard<'_, T>, TryLockError> {
        self.0.try_lock()
    }

    #[track_caller]
    pub fn try_lock_owned(self: Arc<Self>) -> Result<OwnedMutexGuard<T>, TryLockError> {
        let guard = Arc::clone(&self.0).try_lock_owned();
        if guard.is_ok() {
            self.__trace(Location::caller());
        }
        guard
    }
}

impl<T: Default> Default for TraceableMutex<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}
