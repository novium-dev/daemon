use crate::Daemon;
use crate::storage::StorageIdentifier;
use crate::sync::mutex::TraceableMutex;
use crate::sync::rwlock::TraceableRwLock;
use async_trait::async_trait;
use daemon_common::service::BlankError;
use daemon_common::service::debug::{
    DaemonTraceResult, DebugService, LastMutexTrace, LastRwLockTrace, ServerTraceResult,
    TraceResult,
};
use std::ops::Deref;

#[async_trait]
impl DebugService for Daemon {
    async fn debug_trace_daemon(&self) -> Result<DaemonTraceResult, BlankError> {
        Ok(DaemonTraceResult {
            servers: generic_mutex_trace(self.servers.deref()),
            storages: generic_mutex_trace(self.storages.deref()),
            schedules: generic_mutex_trace(self.schedules.deref()),
        })
    }

    async fn debug_trace_server(&self, server_id: u32) -> Result<ServerTraceResult, BlankError> {
        let (data, buffer, attach_write, sessions) = match self.servers.try_lock_hidden() {
            Ok(servers) => {
                let server = servers.get(&server_id).cloned();
                drop(servers);
                match server {
                    None => (None, None, None, None),
                    Some(server_data) => {
                        let (buffer, attach_write, sessions) = match server_data.try_lock_hidden() {
                            Ok(data) => (
                                Some(generic_mutex_trace(data.buffer.deref())),
                                Some(generic_mutex_trace(data.attach_write.deref())),
                                Some(generic_mutex_trace(data.sessions.deref())),
                            ),
                            Err(_) => (None, None, None),
                        };
                        (
                            Some(generic_mutex_trace(server_data.deref())),
                            buffer,
                            attach_write,
                            sessions,
                        )
                    }
                }
            }
            Err(_) => (None, None, None, None),
        };
        let (storage, storage_stats) = match self.storages.try_lock_hidden() {
            Ok(storages) => {
                let storage = storages
                    .get(&StorageIdentifier::Primary(server_id))
                    .cloned();
                drop(storages);
                match storage {
                    None => (None, None),
                    Some(storage_data) => (
                        Some(generic_rwlock_trace(storage_data.lock.deref())),
                        Some(generic_mutex_trace(storage_data.stats_lock.deref())),
                    ),
                }
            }
            Err(_) => (None, None),
        };
        let schedule = match self.schedules.try_lock_hidden() {
            Ok(schedules) => schedules
                .get(&server_id)
                .cloned()
                .map(|schedule_data| generic_mutex_trace(schedule_data.deref())),
            Err(_) => None,
        };

        Ok(ServerTraceResult {
            data,
            buffer,
            attach_write,
            sessions,
            storage,
            storage_stats,
            schedule,
        })
    }
}

fn generic_mutex_trace<T>(mutex: &TraceableMutex<T>) -> TraceResult<LastMutexTrace> {
    let trace = mutex.last_trace();
    TraceResult {
        last_trace: trace.map(|(file, line, column, time)| LastMutexTrace {
            file: file.to_string(),
            line,
            column,
            time: time.to_rfc3339(),
        }),
        locked: mutex.try_lock_hidden().is_err(),
    }
}

fn generic_rwlock_trace<T>(mutex: &TraceableRwLock<T>) -> TraceResult<LastRwLockTrace> {
    let trace = mutex.last_trace();
    TraceResult {
        last_trace: trace.map(|(file, line, column, time, write)| LastRwLockTrace {
            file: file.to_string(),
            line,
            column,
            time: time.to_rfc3339(),
            write,
        }),
        locked: mutex.try_write_hidden().is_err(),
    }
}
