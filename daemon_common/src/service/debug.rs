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
#![expect(
    clippy::option_if_let_else,
    reason = "utoipa doesn't use #[automatically_derived], see https://github.com/juhaku/utoipa/pull/1402"
)]

use super::*;
use daemon_framework::service;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TraceResult<T> {
    pub last_trace: Option<T>,
    pub locked: bool,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LastMutexTrace {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub time: String,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct LastRwLockTrace {
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub time: String,
    pub write: bool,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
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
pub struct DaemonTraceResult {
    pub servers: TraceResult<LastMutexTrace>,
    pub storages: TraceResult<LastMutexTrace>,
    pub schedules: TraceResult<LastMutexTrace>,
}

#[service(path = "/debug", tag = "debug")]
pub trait DebugService {
    /// Print debug information about the synchronization primitives used in the daemon
    #[route("/trace", method = GET, no_auth)]
    async fn debug_trace_daemon(&self) -> Result<DaemonTraceResult, BlankError>;

    /// Print debug information about the synchronization primitives used in a server
    #[route("/trace/{server_id}", method = GET, no_auth)]
    async fn debug_trace_server(&self, server_id: u32) -> Result<ServerTraceResult, BlankError>;
}
