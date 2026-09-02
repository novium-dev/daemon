// SPDX-License-Identifier: AGPL-3.0-only
use super::*;
use chrono_tz::Tz;
use cron::Schedule;
use serde::{Deserialize, Serialize};
use utoipa::PartialSchema;
use utoipa::openapi::{Object, RefOr, Schema};
use zelus::service;

define_path_variable!(schedule_id "The schedule id");
define_error!(schedule {
    not_found ("The schedule does not exist" NOT_FOUND),
});

#[service(path = "/schedule/{server_id}", tag = "schedule")]
pub trait ScheduleService {
    /// Lists every schedule of an existing server
    #[route("", method = GET)]
    #[error(server::not_found, sql::failure <- [sql:*])]
    async fn list_schedules(&self, server_id: u32) -> Result<Vec<ScheduleResponse>, _>;

    /// Creates a new schedule
    #[route("", method = PUT)]
    #[error(server::not_found, sql::failure <- [sql:*])]
    #[example("examples/schedule.json")]
    async fn create_schedule(&self, server_id: u32, payload: SchedulePayload) -> Result<u32, _>;

    /// Updates a schedule
    #[route("/{schedule_id}", method = PATCH)]
    #[error(server::not_found, schedule::not_found, sql::failure <- [sql:*])]
    #[example("examples/schedule.json")]
    async fn update_schedule(
        &self,
        server_id: u32,
        schedule_id: u32,
        payload: SchedulePayload,
    ) -> Result<(), _>;

    /// Deletes a schedule
    #[route("/{schedule_id}", method = DELETE)]
    #[error(server::not_found, sql::failure <- [sql:*])]
    async fn delete_schedule(&self, server_id: u32, schedule_id: u32) -> Result<(), _>;
}

#[derive(Clone, Serialize, Deserialize, Debug, utoipa::ToSchema)]
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
pub enum ScheduleRunPolicy {
    #[serde(rename = "always")]
    Always,
    #[serde(rename = "running")]
    Running,
    #[serde(rename = "stopped")]
    Stopped,
}

impl ScheduleRunPolicy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Running => "running",
            Self::Stopped => "stopped",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "always" => Some(Self::Always),
            "running" => Some(Self::Running),
            "stopped" => Some(Self::Stopped),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SchedulePayload {
    pub name: String,
    pub enabled: bool,
    pub policy: ScheduleRunPolicy,
    pub schedule: Schedule,
    pub timezone: Tz,
    pub steps: Vec<ScheduleStep>,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ScheduleResponse {
    pub id: u32,
    pub name: String,
    pub enabled: bool,
    pub policy: ScheduleRunPolicy,
    pub schedule: String,
    pub timezone: String,
    pub steps: Vec<ScheduleStep>,
}

impl PartialSchema for SchedulePayload {
    fn schema() -> RefOr<Schema> {
        RefOr::T(Schema::Object(
            Object::builder()
                .property("name", String::schema())
                .property("enabled", bool::schema())
                .property("policy", ScheduleRunPolicy::schema())
                .property("schedule", String::schema())
                .property("timezone", String::schema())
                .property("steps", <Vec<ScheduleStep>>::schema())
                .required("name")
                .required("enabled")
                .required("policy")
                .required("schedule")
                .required("timezone")
                .required("steps")
                .build(),
        ))
    }
}

impl utoipa::ToSchema for SchedulePayload {}
