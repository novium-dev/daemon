// SPDX-License-Identifier: AGPL-3.0-only

use serde::Serialize;

#[derive(Clone, Copy, Serialize)]
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
