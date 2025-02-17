use std::future::Future;
use tokio::sync::{Mutex as TokioMutex, MutexGuard, OwnedMutexGuard, TryLockError};

pub type MutexTraceData = Option<(&'static str, u32, u32, chrono::DateTime<chrono::Utc>)>;
pub(crate) struct TraceableMutex<T: ?Sized>(
    std::sync::Arc<TokioMutex<T>>,
    std::sync::Mutex<MutexTraceData>,
);

#[allow(dead_code)]
impl<T: ?Sized> TraceableMutex<T> {
    pub fn new(value: T) -> Self
    where
        T: Sized,
    {
        Self(
            std::sync::Arc::new(tokio::sync::Mutex::new(value)),
            std::sync::Mutex::default(),
        )
    }

    pub(crate) fn last_trace(&self) -> MutexTraceData {
        *self.1.lock().unwrap()
    }

    fn __trace(&self, location: &'static std::panic::Location<'static>) {
        let file = location.file();
        let line = location.line();
        let column = location.column();
        let time = chrono::Utc::now();
        *self.1.lock().unwrap() = Some((file, line, column, time));
    }

    #[track_caller]
    pub(crate) fn lock(&self) -> impl Future<Output = MutexGuard<'_, T>> {
        let location = std::panic::Location::caller();
        async move {
            let guard = self.0.lock().await;
            self.__trace(location);
            guard
        }
    }

    #[track_caller]
    pub(crate) fn lock_owned(
        self: std::sync::Arc<Self>,
    ) -> impl Future<Output = OwnedMutexGuard<T>> {
        let location = std::panic::Location::caller();
        async move {
            let guard = self.0.clone().lock_owned().await;
            self.__trace(location);
            guard
        }
    }

    #[track_caller]
    pub(crate) fn blocking_lock(&self) -> MutexGuard<'_, T> {
        let guard = self.0.blocking_lock();
        self.__trace(std::panic::Location::caller());
        guard
    }

    #[track_caller]
    pub(crate) fn blocking_lock_owned(self: std::sync::Arc<Self>) -> OwnedMutexGuard<T> {
        let guard = self.0.clone().blocking_lock_owned();
        self.__trace(std::panic::Location::caller());
        guard
    }

    #[track_caller]
    pub(crate) fn try_lock(&self) -> Result<MutexGuard<'_, T>, TryLockError> {
        let guard = self.0.try_lock();
        if guard.is_ok() {
            self.__trace(std::panic::Location::caller());
        }
        guard
    }

    pub(crate) fn try_lock_hidden(&self) -> Result<MutexGuard<'_, T>, TryLockError> {
        self.0.try_lock()
    }

    #[track_caller]
    pub(crate) fn try_lock_owned(
        self: std::sync::Arc<Self>,
    ) -> Result<OwnedMutexGuard<T>, TryLockError> {
        let guard = self.0.clone().try_lock_owned();
        if guard.is_ok() {
            self.__trace(std::panic::Location::caller());
        }
        guard
    }
}

impl<T: Default> Default for TraceableMutex<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}
