use crate::Daemon;
use crate::storage::StorageIdentifier;
use crate::sync::mutex::TraceableMutex;
use crate::sync::rwlock::TraceableRwLock;
use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use std::ops::Deref;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

#[derive(Debug, Serialize)]
pub(crate) struct TraceResult<T> {
    last_trace: Option<T>,
    locked: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct LastMutexTrace {
    file: &'static str,
    line: u32,
    column: u32,
    time: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct LastRwLockTrace {
    file: &'static str,
    line: u32,
    column: u32,
    time: String,
    write: bool,
}

#[derive(Debug, Serialize)]
struct ServerTraceResult {
    data: Option<TraceResult<LastMutexTrace>>,
    buffer: Option<TraceResult<LastMutexTrace>>,
    attach_write: Option<TraceResult<LastMutexTrace>>,
    sessions: Option<TraceResult<LastMutexTrace>>,
    storage: Option<TraceResult<LastRwLockTrace>>,
    storage_stats: Option<TraceResult<LastMutexTrace>>,
    schedule: Option<TraceResult<LastMutexTrace>>,
}

#[derive(Debug, Serialize)]
struct DaemonTraceResult {
    servers: TraceResult<LastMutexTrace>,
    storages: TraceResult<LastMutexTrace>,
    schedules: TraceResult<LastMutexTrace>,
}

pub(crate) fn config() -> OpenApiRouter<Daemon> {
    OpenApiRouter::new()
        .routes(routes!(daemon_trace))
        .routes(routes!(server_trace))
}

#[utoipa::path(get,
    path = "/trace",
    tag = "trace",
    description = "Trace the daemon",
    responses(
        (status = 200, description = "Trace result"),
    ),
)]
async fn daemon_trace(State(state): State<Daemon>) -> Json<DaemonTraceResult> {
    Json(DaemonTraceResult {
        servers: generic_mutex_trace(state.servers.deref()),
        storages: generic_mutex_trace(state.storages.deref()),
        schedules: generic_mutex_trace(state.schedules.deref()),
    })
}

#[utoipa::path(get,
    path = "/trace/server/{server_id}",
    tag = "trace",
    description = "Traces an existing server",
    responses(
        (status = 200, description = "Trace result"),
    ),
    params(
        ("server_id" = u32, description = "The id of the server"),
   )
)]
async fn server_trace(
    State(state): State<Daemon>,
    Path((server_id,)): Path<(u32,)>,
) -> Json<ServerTraceResult> {
    let (data, buffer, attach_write, sessions) = match state.servers.try_lock_hidden() {
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
    let (storage, storage_stats) = match state.storages.try_lock_hidden() {
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
    let schedule = match state.schedules.try_lock_hidden() {
        Ok(schedules) => schedules
            .get(&server_id)
            .cloned()
            .map(|schedule_data| generic_mutex_trace(schedule_data.deref())),
        Err(_) => None,
    };

    Json(ServerTraceResult {
        data,
        buffer,
        attach_write,
        sessions,
        storage,
        storage_stats,
        schedule,
    })
}

fn generic_mutex_trace<T>(mutex: &TraceableMutex<T>) -> TraceResult<LastMutexTrace> {
    let trace = mutex.last_trace();
    TraceResult {
        last_trace: trace.map(|(file, line, column, time)| LastMutexTrace {
            file,
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
            file,
            line,
            column,
            time: time.to_rfc3339(),
            write,
        }),
        locked: mutex.try_write_hidden().is_err(),
    }
}
