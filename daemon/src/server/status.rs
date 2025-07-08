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
use crate::ServerData;
use crate::server::FetchedServer;
use crate::sync::Mutex;
use crate::utils::ChannelWriteData;
use axum::extract::ws::Message;
use bollard::Docker;
use bollard::models::HealthStatusEnum;
use bollard::query_parameters::{InspectContainerOptions, ListContainersOptions};
use chrono::DateTime;
use core::fmt::{Display, Formatter};
use daemon_common::service::server::PublicServerState;
use serde::{Serialize, Serializer};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;
use tokio::sync::broadcast::Sender;
use utoipa::openapi::{RefOr, Schema};
use utoipa::{PartialSchema, ToSchema};

impl FetchedServer {
    pub(crate) async fn update_status(
        id: u32,
        locks: HashSet<String>,
        stats_sender: &Sender<ChannelWriteData>,
        docker: &Docker,
        server_data_arc: &Arc<Mutex<ServerData>>,
    ) {
        if stats_sender.receiver_count() == 0 {
            return;
        }
        let server_data = server_data_arc.lock().await;
        Self::update_status0(id, locks, docker, &server_data).await;
    }

    pub(crate) async fn update_status0(
        id: u32,
        locks: HashSet<String>,
        docker: &Docker,
        server_data: &ServerData,
    ) {
        if let Ok(state) = Self::state(id, locks, docker, Some(server_data)).await {
            let _ = server_data
                .stats_sender
                .send(ChannelWriteData::Status(state));
        }
    }

    pub(crate) async fn state(
        id: u32,
        locks: HashSet<String>,
        docker: &Docker,
        server_data: Option<&ServerData>,
    ) -> Result<InternalServerState, bollard::errors::Error> {
        let list = docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(HashMap::from([(
                    "name".to_owned(),
                    vec![Self::container_name(id)],
                )])),
                ..ListContainersOptions::default()
            }))
            .await?;
        if list.is_empty() {
            Ok(if locks.is_empty() {
                InternalServerState::Stopped
            } else {
                InternalServerState::Locked
            })
        } else {
            let inspect = docker
                .inspect_container(&Self::container_name(id), None::<InspectContainerOptions>)
                .await?;
            let created = inspect
                .created
                .map(|time| DateTime::parse_from_rfc3339(&time).ok())
                .unwrap_or_default()
                .map(|time| time.to_utc().timestamp_millis())
                .unwrap_or_default();

            let Some(state) = inspect.state else {
                return Err(bollard::errors::Error::IOError {
                    err: io::Error::other("state is missing"),
                });
            };
            let Some(running) = state.running else {
                return Err(bollard::errors::Error::IOError {
                    err: io::Error::other("state.running is missing"),
                });
            };
            Ok(if !running {
                if locks.is_empty() {
                    InternalServerState::Stopped
                } else {
                    InternalServerState::Locked
                }
            } else if let Some(server_data) = server_data
                && server_data.stopping
            {
                if server_data.restart {
                    InternalServerState::Restarting(created)
                } else {
                    InternalServerState::Stopping(created)
                }
            } else if let Some(mut health) = state.health {
                let healthcheck = health
                    .log
                    .as_mut()
                    .map(Vec::pop)
                    .unwrap_or_default()
                    .map(|result| result.output)
                    .unwrap_or_default();

                match health.status {
                    Some(HealthStatusEnum::STARTING) => {
                        InternalServerState::Starting(created, healthcheck)
                    }
                    Some(
                        HealthStatusEnum::HEALTHY
                        | HealthStatusEnum::EMPTY
                        | HealthStatusEnum::NONE,
                    )
                    | None => InternalServerState::Running(created, healthcheck),
                    Some(HealthStatusEnum::UNHEALTHY) => {
                        InternalServerState::Unhealthy(created, healthcheck)
                    }
                }
            } else {
                InternalServerState::Running(created, None)
            })
        }
    }
}

#[derive(Clone)]
pub enum InternalServerState {
    Stopped,

    // Health-check enabled
    Starting(i64, Option<String>),
    Unhealthy(i64, Option<String>),

    Running(i64, Option<String>),

    Stopping(i64),
    Restarting(i64),

    Locked,
}

impl PartialSchema for InternalServerState {
    fn schema() -> RefOr<Schema> {
        PublicServerState::schema()
    }
}

impl ToSchema for InternalServerState {
    fn name() -> Cow<'static, str> {
        PublicServerState::name()
    }
}

impl InternalServerState {
    pub const fn start(&self) -> bool {
        matches!(self, Self::Stopped)
    }

    pub const fn stop(&self) -> bool {
        !matches!(
            self,
            Self::Stopped | Self::Stopping(_) | Self::Restarting(_) | Self::Locked
        )
    }

    pub const fn kill(&self) -> bool {
        !matches!(self, Self::Stopped | Self::Locked)
    }

    pub const fn kill_show(&self) -> bool {
        matches!(self, Self::Stopping(_) | Self::Restarting(_))
    }

    pub const fn since(&self) -> Option<i64> {
        match self {
            Self::Restarting(time)
            | Self::Running(time, _)
            | Self::Starting(time, _)
            | Self::Stopping(time)
            | Self::Unhealthy(time, _) => Some(*time),
            _ => None,
        }
    }

    pub(crate) fn public(&self) -> PublicServerState {
        PublicServerState {
            label: self.to_string(),
            healthcheck: match self {
                Self::Starting(_, healthcheck)
                | Self::Unhealthy(_, healthcheck)
                | Self::Running(_, healthcheck) => healthcheck,
                _ => &None,
            }
            .clone(),
            start: self.start(),
            stop: self.stop(),
            kill: self.kill_show(),
            time: self.since(),
        }
    }
}

impl Display for InternalServerState {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Stopped => "Stopped",
            Self::Starting(_, _) => "Starting",
            Self::Unhealthy(_, _) => "Unhealthy",
            Self::Running(_, _) => "Running",
            Self::Stopping(_) => "Stopping",
            Self::Restarting(_) => "Restarting",
            Self::Locked => "Locked",
        })
    }
}

impl InternalServerState {
    pub fn as_websocket_message(&self) -> Message {
        Message::Text(serde_json::to_string(self).unwrap_or_default().into())
    }
}

impl Serialize for InternalServerState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.public().serialize(serializer)
    }
}
