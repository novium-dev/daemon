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
    /// Creates a new schedule
    #[route("", method = PUT)]
    #[error(server(not_found) sql(failure [sql:*]))]
    #[example("examples/schedule.json")]
    async fn create_schedule(&self, server_id: u32, payload: SchedulePayload) -> Result<u32, _>;

    /// Updates a schedule
    #[route("/{schedule_id}", method = PATCH)]
    #[error(server(not_found) schedule(not_found) sql(failure [sql:*]))]
    #[example("examples/schedule.json")]
    async fn update_schedule(
        &self,
        server_id: u32,
        schedule_id: u32,
        payload: SchedulePayload,
    ) -> Result<(), _>;

    /// Deletes a schedule
    #[route("/{schedule_id}", method = DELETE)]
    #[error(server(not_found) sql(failure [sql:*]))]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SchedulePayload {
    pub policy: ScheduleRunPolicy,
    pub schedule: Schedule,
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
