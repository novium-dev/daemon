use chrono_tz::Tz;
use cron::Schedule;
use daemon_framework::ts;
use serde::{Deserialize, Serialize};
use utoipa::PartialSchema;
use utoipa::openapi::{Object, RefOr, Schema};

#[derive(Clone, Serialize, Deserialize, Debug, utoipa::ToSchema)]
#[ts]
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
    Sleep { time: u64 },
    #[serde(rename = "terminal")]
    TerminalCommand { command: String },
    #[serde(rename = "container")]
    ContainerCommand { command: Vec<String> },
}

#[derive(Clone, Copy, Hash, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[ts]
pub enum ScheduleRunPolicy {
    #[serde(rename = "always")]
    Always,
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "stopped")]
    Stopped,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[ts]
pub struct SchedulePayload {
    pub policy: ScheduleRunPolicy,
    #[ts(type = "string")]
    pub schedule: Schedule,
    #[ts(type = "string")]
    pub timezone: Tz,
    pub steps: Vec<ScheduleStep>,
}

impl PartialSchema for SchedulePayload {
    fn schema() -> RefOr<Schema> {
        RefOr::T(Schema::Object(
            Object::builder()
                .property("policy", ScheduleRunPolicy::schema())
                .property("schedule", String::schema())
                .property("timezone", String::schema())
                .property("steps", <Vec<ScheduleStep>>::schema())
                .required("policy")
                .required("schedule")
                .required("timezone")
                .required("steps")
                .build(),
        ))
    }
}

impl utoipa::ToSchema for SchedulePayload {}
