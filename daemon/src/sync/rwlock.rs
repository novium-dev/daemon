use std::future::Future;
use std::sync::Arc;
use tokio::sync::{
    OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock as TokioRwLock, RwLockReadGuard,
    RwLockWriteGuard, TryLockError,
};

pub type RwLockTraceData = Option<(&'static str, u32, u32, chrono::DateTime<chrono::Utc>, bool)>;
pub(crate) struct TraceableRwLock<T: ?Sized>(
    std::sync::Arc<TokioRwLock<T>>,
    std::sync::Mutex<RwLockTraceData>,
);

#[allow(dead_code)]
impl<T: ?Sized> TraceableRwLock<T> {
    pub fn new(value: T) -> Self
    where
        T: Sized,
    {
        Self(
            std::sync::Arc::new(tokio::sync::RwLock::new(value)),
            std::sync::Mutex::default(),
        )
    }

    pub(crate) fn last_trace(&self) -> RwLockTraceData {
        *self.1.lock().unwrap()
    }

    fn __trace(&self, location: &'static std::panic::Location<'static>, write: bool) {
        let file = location.file();
        let line = location.line();
        let column = location.column();
        let time = chrono::Utc::now();
        *self.1.lock().unwrap() = Some((file, line, column, time, write));
    }

    #[track_caller]
    pub(crate) fn read(&self) -> impl Future<Output = RwLockReadGuard<'_, T>> {
        let location = std::panic::Location::caller();
        async move {
            let guard = self.0.read().await;
            self.__trace(location, false);
            guard
        }
    }

    #[track_caller]
    pub(crate) fn read_owned(
        self: std::sync::Arc<Self>,
    ) -> impl Future<Output = OwnedRwLockReadGuard<T>> {
        let location = std::panic::Location::caller();
        async move {
            let guard = self.0.clone().read_owned().await;
            self.__trace(location, false);
            guard
        }
    }

    #[track_caller]
    pub(crate) fn blocking_read(&self) -> RwLockReadGuard<'_, T> {
        let guard = self.0.blocking_read();
        self.__trace(std::panic::Location::caller(), false);
        guard
    }

    #[track_caller]
    pub(crate) fn try_read(&self) -> Result<RwLockReadGuard<'_, T>, TryLockError> {
        let guard = self.0.try_read();
        if guard.is_ok() {
            self.__trace(std::panic::Location::caller(), false);
        }
        guard
    }

    #[track_caller]
    pub(crate) fn try_read_owned(self: Arc<Self>) -> Result<OwnedRwLockReadGuard<T>, TryLockError> {
        let guard = self.0.clone().try_read_owned();
        if guard.is_ok() {
            self.__trace(std::panic::Location::caller(), false);
        }
        guard
    }

    #[track_caller]
    pub(crate) fn write(&self) -> impl Future<Output = RwLockWriteGuard<'_, T>> {
        let location = std::panic::Location::caller();
        async move {
            let guard = self.0.write().await;
            self.__trace(location, true);
            guard
        }
    }

    #[track_caller]
    pub(crate) fn write_owned(
        self: std::sync::Arc<Self>,
    ) -> impl Future<Output = OwnedRwLockWriteGuard<T>> {
        let location = std::panic::Location::caller();
        async move {
            let guard = self.0.clone().write_owned().await;
            self.__trace(location, true);
            guard
        }
    }

    #[track_caller]
    pub(crate) fn blocking_write(&self) -> RwLockWriteGuard<'_, T> {
        let guard = self.0.blocking_write();
        self.__trace(std::panic::Location::caller(), true);
        guard
    }

    #[track_caller]
    pub(crate) fn try_write(&self) -> Result<RwLockWriteGuard<'_, T>, TryLockError> {
        let guard = self.0.try_write();
        if guard.is_ok() {
            self.__trace(std::panic::Location::caller(), true);
        }
        guard
    }

    #[track_caller]
    pub(crate) fn try_write_owned(
        self: Arc<Self>,
    ) -> Result<OwnedRwLockWriteGuard<T>, TryLockError> {
        let guard = self.0.clone().try_write_owned();
        if guard.is_ok() {
            self.__trace(std::panic::Location::caller(), true);
        }
        guard
    }

    pub(crate) fn try_read_hidden(&self) -> Result<RwLockReadGuard<'_, T>, TryLockError> {
        self.0.try_read()
    }

    pub(crate) fn try_write_hidden(&self) -> Result<RwLockWriteGuard<'_, T>, TryLockError> {
        self.0.try_write()
    }
}

impl<T: Default> Default for TraceableRwLock<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}
