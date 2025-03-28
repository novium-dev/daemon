use chrono_tz::Tz;
use cron::Schedule;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ScheduleStep {
    #[serde(rename = "start")]
    Start,
    #[serde(rename = "stop")]
    Stop,
    #[serde(rename = "restart")]
    Restart,
    #[serde(rename = "kill")]
    Kill,
    #[serde(rename = "sleep")]
    Sleep { time: Duration },
    #[serde(rename = "terminal")]
    TerminalCommand { command: String },
    #[serde(rename = "container")]
    ContainerCommand { command: Vec<String> },
}

#[derive(Clone, Copy, Hash, Debug, Serialize, Deserialize)]
pub enum ScheduleRunPolicy {
    #[serde(rename = "always")]
    Always,
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "stopped")]
    Stopped,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SchedulePayload {
    pub policy: ScheduleRunPolicy,
    pub schedule: Schedule,
    pub timezone: Tz,
    pub steps: Vec<ScheduleStep>,
}
