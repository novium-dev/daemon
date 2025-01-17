use crate::server::FetchedServer;
use crate::utils::ChannelWriteData;
use crate::ServerData;
#[cfg(feature = "user")]
use axum::extract::ws::Message;
use bollard::container::ListContainersOptions;
use bollard::models::HealthStatusEnum;
use bollard::Docker;
use chrono::DateTime;
use daemon_common::server::PublicServerState;
use serde::{Serialize, Serializer};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use tokio::sync::broadcast::Sender;
use tokio::sync::Mutex;
use utoipa::openapi::{RefOr, Schema};
use utoipa::{PartialSchema, ToSchema};

impl FetchedServer {
    pub(crate) async fn update_status(
        id: u32,
        stats_sender: &Sender<ChannelWriteData>,
        docker: &Docker,
        server_data_arc: &Arc<Mutex<ServerData>>,
    ) {
        if stats_sender.receiver_count() == 0 {
            return;
        }
        let server_data = server_data_arc.lock().await;
        Self::update_status0(id, docker, &server_data).await;
    }

    pub(crate) async fn update_status0(id: u32, docker: &Docker, server_data: &ServerData) {
        if let Ok(state) = Self::state(id, docker, server_data).await {
            let _ = server_data
                .stats_sender
                .send(ChannelWriteData::Status(state));
        }
    }

    pub(crate) async fn state(
        id: u32,
        docker: &Docker,
        server_data: &ServerData,
    ) -> Result<InternalServerState, bollard::errors::Error> {
        let list = docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: HashMap::from([("name".to_owned(), vec![Self::container_name(id)])]),
                ..ListContainersOptions::default()
            }))
            .await?;
        if list.is_empty() {
            Ok(InternalServerState::Stopped)
        } else {
            let inspect = docker
                .inspect_container(&Self::container_name(id), None)
                .await?;
            let created = inspect
                .created
                .map(|time| DateTime::parse_from_rfc3339(&time).ok())
                .unwrap_or_default()
                .map(|time| time.to_utc().timestamp_millis())
                .unwrap_or_default();

            let state = inspect.state.unwrap();
            Ok(if !state.running.unwrap() {
                InternalServerState::Stopped
            } else if server_data.stopping {
                if server_data.restart {
                    InternalServerState::Restarting(created)
                } else {
                    InternalServerState::Stopping(created)
                }
            } else if let Some(health) = state.health {
                let healthcheck = health
                    .log
                    .map(|mut log| log.pop())
                    .unwrap_or_default()
                    .map(|result| result.output)
                    .unwrap_or_default();

                match health.status.unwrap() {
                    HealthStatusEnum::STARTING => {
                        InternalServerState::Starting(created, healthcheck)
                    }
                    HealthStatusEnum::HEALTHY
                    | HealthStatusEnum::EMPTY
                    | HealthStatusEnum::NONE => InternalServerState::Running(created, healthcheck),
                    HealthStatusEnum::UNHEALTHY => {
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
    pub(crate) fn start(&self) -> bool {
        matches!(self, InternalServerState::Stopped)
    }

    pub(crate) fn stop(&self) -> bool {
        !matches!(
            self,
            InternalServerState::Stopped
                | InternalServerState::Stopping(_)
                | InternalServerState::Restarting(_)
        )
    }

    pub(crate) fn kill(&self) -> bool {
        !matches!(self, InternalServerState::Stopped)
    }

    #[allow(unused)]
    pub(crate) fn kill_show(&self) -> bool {
        matches!(
            self,
            InternalServerState::Stopping(_) | InternalServerState::Restarting(_)
        )
    }

    pub(crate) fn since(&self) -> Option<i64> {
        match self {
            InternalServerState::Restarting(time)
            | InternalServerState::Running(time, _)
            | InternalServerState::Starting(time, _)
            | InternalServerState::Stopping(time)
            | InternalServerState::Unhealthy(time, _) => Some(*time),
            _ => None,
        }
    }
}

impl Display for InternalServerState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            InternalServerState::Stopped => "Stopped",
            InternalServerState::Starting(_, _) => "Starting",
            InternalServerState::Unhealthy(_, _) => "Unhealthy",
            InternalServerState::Running(_, _) => "Running",
            InternalServerState::Stopping(_) => "Stopping",
            InternalServerState::Restarting(_) => "Restarting",
        })
    }
}

#[cfg(feature = "user")]
impl InternalServerState {
    pub fn as_websocket_message(&self) -> Message {
        Message::Text(serde_json::to_string(self).unwrap().into())
    }
}

impl Serialize for InternalServerState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        PublicServerState {
            label: self.to_string(),
            healthcheck: match self {
                InternalServerState::Starting(_, healthcheck)
                | InternalServerState::Unhealthy(_, healthcheck)
                | InternalServerState::Running(_, healthcheck) => healthcheck,
                _ => &None,
            }
            .clone(),
            start: self.start(),
            stop: self.stop(),
            kill: self.kill_show(),
            time: self.since(),
        }
        .serialize(serializer)
    }
}
