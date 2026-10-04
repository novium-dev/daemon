// SPDX-License-Identifier: AGPL-3.0-only
use std::{
    collections::{HashMap, HashSet},
    io,
    sync::Arc,
};

use bollard::query_parameters::{ListContainersOptions, StatsOptions};
use daemon_common::service::resource::ResourceSubset;
use futures_util::StreamExt as _;
use num_traits::ToPrimitive as _;
use sqlx::query;
use sqlx::types::BigDecimal;
use sysinfo::{MINIMUM_CPU_UPDATE_INTERVAL, System};
use tokio::task::JoinSet;
use tracing::error;

use super::{Allocation, CgroupIdentifier, GROUP_LABEL, HostResources, groups_flat};
use crate::{Daemon, DaemonExtra, config::LimitConfig};

#[derive(Clone, Copy, Default)]
struct Usage {
    cpu: f64,
    mem: u64,
}

impl<T: DaemonExtra> Daemon<T> {
    /// macOS has no host cgroups, so group limits are applied to each container on start
    /// instead (see [`Daemon::container_limits`]). This only cleans up unused groups.
    pub async fn sync_cgroups<E: From<sqlx::Error> + From<io::Error> + From<zbus::Error>>(
        &self,
    ) -> Result<(), E> {
        query!(
            r#"DELETE FROM `groups`
            		WHERE
           				`name` NOT IN (SELECT `group` FROM `servers` WHERE `group` IS NOT NULL)
               			AND `cpu` IS NULL
                  		AND `ram` IS NULL
                    	AND `swap` IS NULL"#
        )
        .execute(&*self.conn)
        .await
        .inspect_err(|err| error!("Failed to cleanup groups in database: {err:?}"))?;

        Ok(())
    }

    /// Clamps a container's limits to the limits of its group, its parent groups and the
    /// daemon's server limit.
    pub(crate) async fn container_limits(
        &self,
        group: Option<&str>,
        limit: LimitConfig,
    ) -> Result<LimitConfig, sqlx::Error> {
        let mut limit = min_limit(limit, self.config.limit);
        let Some(group) = group.filter(|group| !group.is_empty()) else {
            return Ok(limit);
        };

        let groups = self.group_limits().await?;
        for name in groups_flat(group.to_owned()) {
            if let Some(group_limit) = groups.get(&name) {
                limit = min_limit(limit, *group_limit);
            }
        }
        Ok(limit)
    }

    pub(crate) async fn host_resources(
        &self,
        names: &HashSet<String>,
        allocation: &Allocation,
    ) -> Result<HostResources, io::Error> {
        let group_limits = self
            .group_limits()
            .await
            .inspect_err(|err| error!("Failed to fetch group limits: {err:?}"))
            .map_err(io::Error::other)?;

        let (containers, (system, cpu_count, mem_total, swap_total)) =
            tokio::join!(self.container_usages(), system_usage());
        let containers = containers
            .inspect_err(|err| error!("Failed to fetch container stats: {err:?}"))
            .map_err(io::Error::other)?;

        let mut server_usage = Usage::default();
        let mut group_usage: HashMap<String, Usage> = HashMap::new();
        for (group, usage) in containers {
            add_usage(&mut server_usage, usage);
            if !group.is_empty() {
                for name in groups_flat(group) {
                    add_usage(group_usage.entry(name).or_default(), usage);
                }
            }
        }

        let total_alloc = (
            allocation.values().map(|alloc| alloc.0).sum(),
            allocation.values().map(|alloc| alloc.1.clone()).sum(),
            allocation.values().map(|alloc| alloc.2.clone()).sum(),
        );

        let groups = names
            .iter()
            .map(|name| {
                let own = group_limits.get(name).copied().unwrap_or_default();
                let effective = groups_flat(name.clone())
                    .iter()
                    .filter_map(|parent| group_limits.get(parent))
                    .fold(min_limit(own, self.config.limit), |acc, parent| {
                        min_limit(acc, *parent)
                    });
                let alloc = allocation
                    .get(&Some(name.clone()))
                    .cloned()
                    .unwrap_or_default();
                (
                    name.clone(),
                    resource_subset(
                        own,
                        effective,
                        group_usage.get(name).copied().unwrap_or_default(),
                        &alloc,
                    ),
                )
            })
            .collect();

        Ok(HostResources {
            cpu_count,
            mem_total,
            swap_total,
            system: ResourceSubset {
                swap_used: system.1,
                ..resource_subset(
                    LimitConfig::default(),
                    LimitConfig::default(),
                    system.0,
                    &total_alloc,
                )
            },
            server: resource_subset(
                self.config.limit,
                self.config.limit,
                server_usage,
                &total_alloc,
            ),
            groups,
        })
    }

    async fn group_limits(&self) -> Result<HashMap<String, LimitConfig>, sqlx::Error> {
        Ok(query!("SELECT * FROM `groups`")
            .fetch_all(&*self.conn)
            .await?
            .into_iter()
            .map(|row| {
                (
                    row.name,
                    LimitConfig {
                        cpu: row.cpu,
                        ram: row.ram,
                        swap: row.swap,
                    },
                )
            })
            .collect())
    }

    async fn container_usages(&self) -> Result<Vec<(String, Usage)>, bollard::errors::Error> {
        let containers = self
            .docker
            .list_containers(Some(ListContainersOptions {
                filters: Some(HashMap::from([(
                    "label".to_owned(),
                    vec![GROUP_LABEL.to_owned()],
                )])),
                ..ListContainersOptions::default()
            }))
            .await?;

        let mut set = JoinSet::new();
        for container in containers {
            let Some(id) = container.id else {
                continue;
            };
            let group = container
                .labels
                .and_then(|mut labels| labels.remove(GROUP_LABEL))
                .unwrap_or_default();
            let docker = Arc::clone(&self.docker);
            set.spawn(async move {
                let stats = docker
                    .stats(
                        &id,
                        Some(StatsOptions {
                            stream: false,
                            one_shot: false,
                        }),
                    )
                    .next()
                    .await;
                (group, stats)
            });
        }

        let mut usages = Vec::new();
        for (group, stats) in set.join_all().await {
            match stats {
                Some(Ok(stats)) => usages.push((group, container_usage(&stats))),
                Some(Err(err)) => error!("Failed to fetch container stats: {err:?}"),
                None => {}
            }
        }
        Ok(usages)
    }
}

pub(crate) const fn cgroup_parent(_id: &CgroupIdentifier<'_>) -> Option<String> {
    None
}

const fn add_usage(target: &mut Usage, usage: Usage) {
    target.cpu += usage.cpu;
    target.mem = target.mem.saturating_add(usage.mem);
}

fn min_limit(left: LimitConfig, right: LimitConfig) -> LimitConfig {
    fn min<V: PartialOrd>(left: Option<V>, right: Option<V>) -> Option<V> {
        match (left, right) {
            (Some(left), Some(right)) => Some(if right < left { right } else { left }),
            (left, None) => left,
            (None, right) => right,
        }
    }
    LimitConfig {
        cpu: min(left.cpu, right.cpu),
        ram: min(left.ram, right.ram),
        swap: min(left.swap, right.swap),
    }
}

fn resource_subset(
    limit: LimitConfig,
    effective: LimitConfig,
    usage: Usage,
    (alloc_cpu, alloc_ram, alloc_swap): &(f64, BigDecimal, BigDecimal),
) -> ResourceSubset {
    ResourceSubset {
        cpu_used: (usage.cpu * 10000f64).round() / 10000f64,
        cpu_allocated: *alloc_cpu,
        cpu_limit: limit.cpu.map(f64::from),
        cpu_limit_effective: effective.cpu.map(f64::from),
        mem_allocated: alloc_ram.to_u64().unwrap_or(u64::MAX),
        mem_limit: limit.ram.map(u64::from),
        mem_limit_effective: effective.ram.map(u64::from),
        mem_used: usage.mem,
        swap_allocated: alloc_swap.to_u64().unwrap_or(u64::MAX),
        swap_limit: limit.swap.map(u64::from),
        swap_limit_effective: effective.swap.map(u64::from),
        swap_used: 0,
    }
}

#[expect(clippy::cast_precision_loss, reason = "cpu doesn't have to be precise")]
fn container_usage(stats: &bollard::models::ContainerStatsResponse) -> Usage {
    let cpu_times = |cpu: Option<&bollard::models::ContainerCpuStats>| {
        cpu.map_or((0, 0, 0), |cpu| {
            (
                cpu.cpu_usage
                    .as_ref()
                    .and_then(|usage| usage.total_usage)
                    .unwrap_or_default(),
                cpu.system_cpu_usage.unwrap_or_default(),
                cpu.online_cpus.unwrap_or_default(),
            )
        })
    };
    let (cpu_now, system_now, cpus) = cpu_times(stats.cpu_stats.as_ref());
    let (cpu_prev, system_prev, _) = cpu_times(stats.precpu_stats.as_ref());
    let system_delta = system_now.saturating_sub(system_prev);

    let cpu = if system_delta == 0 {
        0f64
    } else {
        cpu_now.saturating_sub(cpu_prev) as f64 / system_delta as f64 * f64::from(cpus)
    };

    let mem = stats.memory_stats.as_ref().map_or(0, |memory| {
        let inactive = memory
            .stats
            .as_ref()
            .and_then(|stats| stats.get("inactive_file").copied())
            .unwrap_or_default();
        memory.usage.unwrap_or_default().saturating_sub(inactive)
    });

    Usage {
        cpu,
        mem: mem >> 20u64,
    }
}

async fn system_usage() -> ((Usage, u64), u32, u64, u64) {
    let mut system = System::new();
    system.refresh_cpu_usage();
    tokio::time::sleep(MINIMUM_CPU_UPDATE_INTERVAL).await;
    system.refresh_cpu_usage();
    system.refresh_memory();

    let cpu_count = u32::try_from(system.cpus().len()).unwrap_or_default();

    (
        (
            Usage {
                cpu: f64::from(system.global_cpu_usage()) / 100f64 * f64::from(cpu_count),
                mem: system.used_memory() >> 20u64,
            },
            system.used_swap() >> 20u64,
        ),
        cpu_count,
        system.total_memory() >> 20u64,
        system.total_swap() >> 20u64,
    )
}
