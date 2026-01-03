// SPDX-License-Identifier: AGPL-3.0-only

#[derive(Clone, Copy)]
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
