use crate::storage::StorageIdentifier;
use crate::sync::Mutex;
use crate::sync::mutex::TraceableMutex;
use crate::sync::rwlock::TraceableRwLock;
use crate::{Daemon, DaemonExtra, ServerData};
use async_trait::async_trait;
use daemon_common::service::BlankError;
use daemon_common::service::debug::{
    DaemonTraceResult, DebugService, LastMutexTrace, LastRwLockTrace, ServerTraceResult,
    TraceResult,
};
use std::sync::Arc;

#[async_trait]
impl<T: DaemonExtra> DebugService for Daemon<T> {
    async fn debug_trace_daemon(&self) -> Result<DaemonTraceResult, BlankError> {
        Ok(DaemonTraceResult {
            servers: generic_mutex_trace(&self.servers),
            storages: generic_mutex_trace(&self.storages),
            schedules: generic_mutex_trace(&self.schedules),
        })
    }

    async fn debug_trace_server(&self, server_id: u32) -> Result<ServerTraceResult, BlankError> {
        let (data, buffer, attach_write, sessions) = trace_server0(self, server_id).map_or(
            (None, None, None, None),
            |(server_data, result)| {
                let (buffer, attach_write, sessions) =
                    result.map_or((None, None, None), |(buffer, attach_write, sessions)| {
                        (Some(buffer), Some(attach_write), Some(sessions))
                    });
                (Some(server_data), buffer, attach_write, sessions)
            },
        );

        let (storage, storage_stats) = trace_storage0(self, StorageIdentifier::Primary(server_id))
            .map_or((None, None), |(lock, stats_lock)| {
                (Some(lock), Some(stats_lock))
            });
        let schedule = self
            .schedules
            .try_lock_hidden()
            .map(|schedules| {
                schedules
                    .get(&server_id)
                    .cloned()
                    .as_deref()
                    .map(generic_mutex_trace)
            })
            .unwrap_or_default();

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

type DepthServerTraceResult = (
    TraceResult<LastMutexTrace>, // buffer
    TraceResult<LastMutexTrace>, // attach_write
    TraceResult<LastMutexTrace>, // sessions
);

#[inline(always)]
fn trace_server0(
    daemon: &Daemon<impl DaemonExtra>,
    server_id: u32,
) -> Option<(
    TraceResult<LastMutexTrace>, // server_data
    Option<DepthServerTraceResult>,
)> {
    let servers = daemon.servers.try_lock_hidden().ok()?;
    let server_data = Arc::clone(servers.get(&server_id)?);
    drop(servers);

    let server_data_trace = trace_server1(&server_data);

    Some((generic_mutex_trace(&server_data), server_data_trace))
}

#[inline(always)]
fn trace_server1(
    server_data: &Mutex<ServerData>,
) -> Option<(
    TraceResult<LastMutexTrace>,
    TraceResult<LastMutexTrace>,
    TraceResult<LastMutexTrace>,
)> {
    let server_data = server_data.try_lock_hidden().ok()?;
    Some((
        generic_mutex_trace(&server_data.buffer),
        generic_mutex_trace(&server_data.attach_write),
        generic_mutex_trace(&server_data.sessions),
    ))
}

#[inline(always)]
fn trace_storage0(
    daemon: &Daemon<impl DaemonExtra>,
    storage_id: StorageIdentifier,
) -> Option<(TraceResult<LastRwLockTrace>, TraceResult<LastMutexTrace>)> {
    let storages = daemon.storages.try_lock_hidden().ok()?;
    let storage = storages.get(&storage_id)?.clone();
    drop(storages);
    Some((
        generic_rwlock_trace(&storage.lock),
        generic_mutex_trace(&storage.stats_lock),
    ))
}

fn generic_mutex_trace<T>(mutex: &TraceableMutex<T>) -> TraceResult<LastMutexTrace> {
    let trace = mutex.last_trace();
    TraceResult {
        last_trace: trace.map(|(file, line, column, time)| LastMutexTrace {
            file: file.to_owned(),
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
            file: file.to_owned(),
            line,
            column,
            time: time.to_rfc3339(),
            write,
        }),
        locked: mutex.try_write_hidden().is_err(),
    }
}
