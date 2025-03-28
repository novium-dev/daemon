use std::time::Duration;

mod worker;

#[derive(Clone)]
pub(crate) enum ScheduleStep {
    Start,
    Stop,
    Restart,
    Kill,
    Sleep(Duration),
    TerminalCommand(String),
    ContainerCommand(Vec<String>),
}

#[derive(Clone, Copy, Hash)]
pub(crate) enum ScheduleStepResult {
    Continue,
    Interrupt,
}

#[derive(Clone, Copy, Hash)]
pub(crate) enum ScheduleRunPolicy {
    Always,
    Running,
    Stopped,
}