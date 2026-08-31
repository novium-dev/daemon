// SPDX-License-Identifier: AGPL-3.0-only
use super::{Check, CheckError, IntegrityContext};
use async_trait::async_trait;
use bollard::query_parameters::{ListContainersOptions, ListNetworksOptions};
use daemon_common::integrity::{IntegrityCategory, IntegrityFinding};
use sqlx::query_scalar;
use std::collections::HashSet;

fn strip_slash(name: &str) -> &str {
    name.strip_prefix('/').unwrap_or(name)
}

pub struct DockerReachable;

#[async_trait]
impl Check for DockerReachable {
    fn id(&self) -> &'static str {
        "docker.reachable"
    }

    fn title(&self) -> &'static str {
        "Container engine reachable"
    }

    fn description(&self) -> &'static str {
        "Pings the Docker daemon this node runs its servers on."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Docker
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        match ctx.docker.ping().await {
            Ok(version) => Ok(vec![
                IntegrityFinding::info("The container engine responded to a ping")
                    .with_detail("response", version),
            ]),
            Err(err) => Ok(vec![IntegrityFinding::error(format!(
                "The container engine did not respond: {err}"
            ))]),
        }
    }
}

pub struct DockerContainers;

#[async_trait]
impl Check for DockerContainers {
    fn id(&self) -> &'static str {
        "docker.containers"
    }

    fn title(&self) -> &'static str {
        "Server containers match the database"
    }

    fn description(&self) -> &'static str {
        "Every container owned by the daemon must belong to a server in the daemon database. \
         Containers are removed when they stop and killed on daemon startup, so one that is \
         still here without a server survived both and is keeping ports, volumes and an ip \
         address allocated."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Docker
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        let known: HashSet<u32> = query_scalar!(r#"SELECT `id` AS `id: u32` FROM `servers`"#)
            .fetch_all(&*ctx.conn)
            .await?
            .into_iter()
            .collect();

        let containers = ctx
            .docker
            .list_containers(Some(ListContainersOptions {
                all: true,
                ..ListContainersOptions::default()
            }))
            .await?;

        let mut findings = Vec::new();
        for container in containers {
            for name in container.names.unwrap_or_default() {
                let Some(suffix) = strip_slash(&name).strip_prefix(crate::common::CONTAINER_PREFIX)
                else {
                    continue;
                };
                let Ok(id) = suffix.parse::<u32>() else {
                    findings.push(
                        IntegrityFinding::warning(
                            "A container carries the daemon prefix but no numeric server id",
                        )
                        .with_subject(name.clone()),
                    );
                    continue;
                };
                if !known.contains(&id) {
                    findings.push(
                        IntegrityFinding::error(format!(
                            "Container of server {id} exists, but the server is not in the daemon database"
                        ))
                        .with_subject(strip_slash(&name).to_owned())
                        .with_detail("server", id.to_string())
                        .with_detail("state", container.state.map_or_else(|| "unknown".to_owned(), |state| format!("{state:?}"))),
                    );
                }
            }
        }

        Ok(findings)
    }
}

pub struct DockerNetworks;

#[async_trait]
impl Check for DockerNetworks {
    fn id(&self) -> &'static str {
        "docker.networks"
    }

    fn title(&self) -> &'static str {
        "Networks exist in both the database and the container engine"
    }

    fn description(&self) -> &'static str {
        "Every row in the networks table must have a matching docker network and the other way \
         around. The two shared bridges the daemon attaches servers to must exist as well."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Docker
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        let known: HashSet<u32> = query_scalar!(r#"SELECT `id` AS `id: u32` FROM `networks`"#)
            .fetch_all(&*ctx.conn)
            .await?
            .into_iter()
            .collect();

        let networks = ctx
            .docker
            .list_networks(None::<ListNetworksOptions>)
            .await?;

        let mut present = HashSet::new();
        let mut names = HashSet::new();
        let mut findings = Vec::new();

        for network in networks {
            let Some(name) = network.name else {
                continue;
            };
            names.insert(name.clone());
            let Some(suffix) = name.strip_prefix(crate::common::NETWORK_PREFIX) else {
                continue;
            };
            match suffix.parse::<u32>() {
                Ok(id) => {
                    present.insert(id);
                    if !known.contains(&id) {
                        findings.push(
                            IntegrityFinding::error(format!(
                                "Docker network {id} exists, but there is no matching row in the networks table"
                            ))
                            .with_subject(name),
                        );
                    }
                }
                Err(_err) => findings.push(
                    IntegrityFinding::warning(
                        "A docker network carries the daemon prefix but no numeric id",
                    )
                    .with_subject(name),
                ),
            }
        }

        for id in &known {
            if !present.contains(id) {
                findings.push(
                    IntegrityFinding::error(format!(
                        "Network {id} is registered in the database, but the docker network is missing"
                    ))
                    .with_subject(format!("{}{id}", crate::common::NETWORK_PREFIX)),
                );
            }
        }

        for shared in [
            crate::common::NETWORK_DEFAULT,
            crate::common::NETWORK_INTERNET,
        ] {
            if !names.contains(shared) {
                findings.push(
                    IntegrityFinding::error(
                        "A shared bridge the daemon attaches servers to is missing",
                    )
                    .with_subject(shared.to_owned()),
                );
            }
        }

        Ok(findings)
    }
}
