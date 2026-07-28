// SPDX-License-Identifier: AGPL-3.0-only

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServerTask {
    Start,
    Stop,
    Restart,
    Kill,
    ScheduleTaskRun,
    ScheduleTerminalCommand,
    ContainerCommand,
    Delete,
}
