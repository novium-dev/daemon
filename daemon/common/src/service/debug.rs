use super::*;
use daemon_framework::{service, ts};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[ts]
pub struct TraceResult<T> {
    pub last_trace: Option<T>,
    pub locked: bool,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[ts]
pub struct LastMutexTrace {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub time: String,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[ts]
pub struct LastRwLockTrace {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub time: String,
    pub write: bool,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[ts]
pub struct ServerTraceResult {
    pub data: Option<TraceResult<LastMutexTrace>>,
    pub buffer: Option<TraceResult<LastMutexTrace>>,
    pub attach_write: Option<TraceResult<LastMutexTrace>>,
    pub sessions: Option<TraceResult<LastMutexTrace>>,
    pub storage: Option<TraceResult<LastRwLockTrace>>,
    pub storage_stats: Option<TraceResult<LastMutexTrace>>,
    pub schedule: Option<TraceResult<LastMutexTrace>>,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[ts]
pub struct DaemonTraceResult {
    pub servers: TraceResult<LastMutexTrace>,
    pub storages: TraceResult<LastMutexTrace>,
    pub schedules: TraceResult<LastMutexTrace>,
}

#[service(path = "/debug", tag = "debug")]
pub trait DebugService {
    #[route("/trace", method = GET, user)]
    #[doc = "Print debug information about the synchronization primitives used in the daemon"]
    async fn debug_trace_daemon(&self) -> Result<DaemonTraceResult, BlankError>;

    #[route("/trace/{server_id}", method = GET, user)]
    #[doc = "Print debug information about the synchronization primitives used in a server"]
    async fn debug_trace_server(&self, server_id: u32) -> Result<ServerTraceResult, BlankError>;
}
