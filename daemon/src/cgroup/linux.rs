// SPDX-License-Identifier: AGPL-3.0-only
use core::time::Duration;
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    io,
};

use daemon_common::service::resource::ResourceSubset;
use num_traits::ToPrimitive as _;
use procfs::{CpuInfo, Current as _, Meminfo};
use sqlx::query;
use sqlx::types::BigDecimal;
use tokio::task::JoinSet;
use tokio::time::Instant;
use tracing::error;
use zbus_systemd::systemd1::{SliceProxy, UnitProxy};

use super::{
    Allocation, CgroupIdentifier, DEFAULT_CPU_PERIOD, HostResources, clamp_cpu_limit, groups_flat,
};
use crate::{Daemon, DaemonExtra, config::LimitConfig, utils::from_mib};

impl<T: DaemonExtra> Daemon<T> {
    /// Cancel safe, kinda.
    pub async fn sync_cgroups<E: From<sqlx::Error> + From<io::Error> + From<zbus::Error>>(
        &self,
    ) -> Result<(), E> {
        let mut conn = self.conn.acquire().await?;

        query!(
            r#"DELETE FROM `groups`
            		WHERE
           				`name` NOT IN (SELECT `group` FROM `servers` WHERE `group` IS NOT NULL)
               			AND `cpu` IS NULL
                  		AND `ram` IS NULL
                    	AND `swap` IS NULL"#
        )
        .execute(&mut *conn)
        .await
        .inspect_err(|err| error!("Failed to cleanup groups in database: {err:?}"))?;

        let groups: HashMap<_, _> = query!("SELECT * FROM `groups`")
            .fetch_all(&mut *conn)
            .await
            .inspect_err(|err| error!("Failed to sync groups (select): {err:?}"))?
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
            .collect();

        self.cgroup_apply_limit::<E>(CgroupIdentifier::Server, self.config.limit)
            .await?;

        for (name, limit) in &groups {
            self.cgroup_apply_limit::<E>(CgroupIdentifier::Group(Cow::Borrowed(name)), *limit)
                .await?;
            for entry in groups_flat(name.clone()) {
                if !groups.contains_key(&entry) {
                    self.cgroup_apply_limit::<E>(
                        CgroupIdentifier::Group(Cow::Owned(entry)),
                        LimitConfig::default(),
                    )
                    .await?;
                }
            }
        }

        Ok(())
    }

    async fn cgroup_apply_limit<E: From<zbus::Error>>(
        &self,
        id: CgroupIdentifier<'_>,
        limit: LimitConfig,
    ) -> Result<(), E> {
        let unit = UnitProxy::new(&self.dbus, systemd_unit_path(&id))
            .await
            .inspect_err(|err| error!("Applying cgroup limits failed (proxy): {err:?}"))?;

        let (cpu_quota, cpu_period) = limit
            .cpu
            .and_then(clamp_cpu_limit::<true>)
            .unwrap_or((u64::MAX, DEFAULT_CPU_PERIOD));

        let memory_max = limit.ram.map_or(u64::MAX, from_mib).max(1);
        let swap_max = limit.swap.map_or(u64::MAX, from_mib);

        unit.set_properties(
            true,
            vec![
                ("CPUQuotaPeriodUSec".to_owned(), cpu_period.into()),
                ("CPUQuotaPerSecUSec".to_owned(), cpu_quota.into()),
                ("MemoryMax".to_owned(), memory_max.into()),
                ("MemorySwapMax".to_owned(), swap_max.into()),
            ],
        )
        .await
        .inspect_err(|err| error!("Applying cgroup limits failed (properties): {err:?}"))?;

        Ok(())
    }

    #[expect(
        clippy::unused_async,
        reason = "same signature as the macOS variant, cgroups enforce group limits here"
    )]
    pub(crate) async fn container_limits(
        &self,
        _group: Option<&str>,
        limit: LimitConfig,
    ) -> Result<LimitConfig, sqlx::Error> {
        Ok(limit)
    }

    pub(crate) async fn host_resources(
        &self,
        names: &HashSet<String>,
        allocation: &Allocation,
    ) -> Result<HostResources, io::Error> {
        let mut groups = HashMap::with_capacity(names.len());

        let usage_prev = usage(&self.dbus, names).await.map_err(resource_dbus_err)?;

        tokio::time::sleep(Duration::from_millis(200)).await; // Recommended time by sysinfo

        let usage_now = usage(&self.dbus, names).await.map_err(resource_dbus_err)?;

        let (cpu, mem) = tokio::task::spawn_blocking(|| (CpuInfo::current(), Meminfo::current()))
            .await
            .inspect_err(|err| error!("failed to access cpu and mem info (task): {err:?}"))
            .map_err(io::Error::from)?;

        let cpu = cpu
            .inspect_err(|err| error!("failed to access cpu info: {err:?}"))
            .map_err(io::Error::other)?;
        let mem = mem
            .inspect_err(|err| error!("failed to access mem info: {err:?}"))
            .map_err(io::Error::other)?;

        let cpu_alloc = allocation.values().map(|alloc| alloc.0).sum();
        let ram_alloc: BigDecimal = allocation.values().map(|alloc| alloc.1.clone()).sum();
        let swap_alloc: BigDecimal = allocation.values().map(|alloc| alloc.2.clone()).sum();
        let alloc = (cpu_alloc, ram_alloc, swap_alloc);

        let mut limit_cache = HashMap::with_capacity(names.len().saturating_add(2));

        let system = new_resource_subset(
            &self.dbus,
            CgroupIdentifier::System,
            &mut limit_cache,
            &usage_prev,
            &usage_now,
            &alloc,
        )
        .await
        .map_err(resource_dbus_err)?;

        let server = new_resource_subset(
            &self.dbus,
            CgroupIdentifier::Server,
            &mut limit_cache,
            &usage_prev,
            &usage_now,
            &alloc,
        )
        .await
        .map_err(resource_dbus_err)?;

        for name in names {
            let id = CgroupIdentifier::Group(Cow::Borrowed(name));
            let alloc = allocation
                .get(&Some(name.to_owned()))
                .cloned()
                .unwrap_or_default();
            groups.insert(
                name.to_owned(),
                new_resource_subset(
                    &self.dbus,
                    id,
                    &mut limit_cache,
                    &usage_prev,
                    &usage_now,
                    &alloc,
                )
                .await
                .map_err(resource_dbus_err)?,
            );
        }

        Ok(HostResources {
            cpu_count: u32::try_from(cpu.num_cores()).unwrap_or_default(),
            mem_total: mem.mem_total >> 20u64,
            swap_total: mem.swap_total >> 20u64,
            system,
            server,
            groups,
        })
    }
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "same signature as the macOS variant, which has no cgroup parent"
)]
pub(crate) fn cgroup_parent(id: &CgroupIdentifier<'_>) -> Option<String> {
    Some(systemd_slice_handle(id))
}

#[inline]
pub(crate) fn systemd_unit_path(id: &CgroupIdentifier<'_>) -> String {
    format!(
        "/org/freedesktop/systemd1/unit/{}",
        systemd_slice_handle(id)
            .replace('_', "_5f")
            .replace('-', "_2d")
            .replace('.', "_2e")
    )
}

#[inline]
pub(crate) fn systemd_slice_handle(group: &CgroupIdentifier<'_>) -> String {
    match group {
        CgroupIdentifier::Group(name) => {
            format!("{}-{name}.slice", crate::common::DAEMON_CGROUP_ROOT)
        }
        CgroupIdentifier::Server => format!("{}.slice", crate::common::DAEMON_CGROUP_ROOT),
        CgroupIdentifier::System => "-.slice".to_owned(),
    }
}

fn resource_dbus_err(err: zbus::Error) -> io::Error {
    error!("failed to request resource info via dbus: {err:?}");
    io::Error::other(err)
}

async fn usage<'a>(
    dbus: &zbus::Connection,
    groups: &'a HashSet<String>,
) -> Result<HashMap<CgroupIdentifier<'a>, (u64, Instant)>, zbus::Error> {
    let mut set = JoinSet::new();

    let conn = dbus.clone();
    set.spawn(async move { usage_time(&conn, CgroupIdentifier::System).await });
    let conn = dbus.clone();
    set.spawn(async move { usage_time(&conn, CgroupIdentifier::Server).await });
    for name in groups {
        let name = name.clone();
        let conn = dbus.clone();
        set.spawn(
            async move { usage_time(&conn, CgroupIdentifier::Group(Cow::Owned(name))).await },
        );
    }

    set.join_all()
        .await
        .into_iter()
        .map(|entry| {
            let entry = entry?;
            Ok((entry.0, (entry.1, entry.2)))
        })
        .collect()
}

#[expect(clippy::cast_precision_loss, reason = "cpu doesn't have to be precise")]
async fn new_resource_subset<'a>(
    conn: &zbus::Connection,
    id: CgroupIdentifier<'a>,
    limit_cache: &mut HashMap<CgroupIdentifier<'a>, (u64, u64, u64)>,
    cpu_usage_prev: &HashMap<CgroupIdentifier<'a>, (u64, Instant)>,
    cpu_usage_new: &HashMap<CgroupIdentifier<'a>, (u64, Instant)>,
    (alloc_cpu, alloc_ram, alloc_swap): &(f64, BigDecimal, BigDecimal),
) -> Result<ResourceSubset, zbus::Error> {
    let cpu_used = if let Some(cpu_prev) = cpu_usage_prev.get(&id)
        && let Some(cpu_new) = cpu_usage_new.get(&id)
    {
        let cpu_delta = cpu_new.0.saturating_sub(cpu_prev.0) as f64;
        let system_delta = cpu_new.1.duration_since(cpu_prev.1).as_micros() as f64;
        ((cpu_delta / system_delta) * 10000f64).round() / 10000f64
    } else {
        0f64
    };

    let mut cpu_limit = None;
    let mut mem_limit = None;
    let mut swap_limit = None;

    apply_limit(
        conn,
        id.clone(),
        limit_cache,
        &mut cpu_limit,
        &mut mem_limit,
        &mut swap_limit,
    )
    .await?;

    let mut cpu_limit_effective = cpu_limit;
    let mut mem_limit_effective = mem_limit;
    let mut swap_limit_effective = swap_limit;

    if let CgroupIdentifier::Group(name) = &id {
        for subgroup in groups_flat(name.to_string()) {
            apply_limit(
                conn,
                CgroupIdentifier::Group(Cow::Owned(subgroup)),
                limit_cache,
                &mut cpu_limit_effective,
                &mut mem_limit_effective,
                &mut swap_limit_effective,
            )
            .await?;
        }
    }

    if !matches!(id, CgroupIdentifier::System) {
        apply_limit(
            conn,
            CgroupIdentifier::System,
            limit_cache,
            &mut cpu_limit_effective,
            &mut mem_limit_effective,
            &mut swap_limit_effective,
        )
        .await?;
    }
    if !matches!(id, CgroupIdentifier::Server | CgroupIdentifier::System) {
        apply_limit(
            conn,
            CgroupIdentifier::Server,
            limit_cache,
            &mut cpu_limit_effective,
            &mut mem_limit_effective,
            &mut swap_limit_effective,
        )
        .await?;
    }

    let proxy = SliceProxy::new(conn, systemd_unit_path(&id)).await?;

    let mem_used = proxy.memory_current().await?;
    let swap_used = proxy.memory_swap_current().await?;

    Ok(ResourceSubset {
        cpu_used,
        cpu_allocated: *alloc_cpu,
        cpu_limit: cpu_limit.map(|limit| limit as f64 / 1_000_000f64),
        cpu_limit_effective: cpu_limit_effective.map(|limit| limit as f64 / 1_000_000f64),
        mem_allocated: alloc_ram.to_u64().unwrap_or(u64::MAX),
        mem_limit: mem_limit.map(|limit| limit >> 20u32),
        mem_limit_effective: mem_limit_effective.map(|limit| limit >> 20u32),
        mem_used: if mem_used == u64::MAX {
            0
        } else {
            mem_used >> 20u32
        },
        swap_allocated: alloc_swap.to_u64().unwrap_or(u64::MAX),
        swap_limit: swap_limit.map(|limit| limit >> 20u32),
        swap_limit_effective: swap_limit_effective.map(|limit| limit >> 20u32),
        swap_used: if swap_used == u64::MAX {
            0
        } else {
            swap_used >> 20u32
        },
    })
}

#[inline]
async fn apply_limit<'a>(
    conn: &zbus::Connection,
    id: CgroupIdentifier<'a>,
    cache: &mut HashMap<CgroupIdentifier<'a>, (u64, u64, u64)>,
    cpu: &mut Option<u64>,
    mem: &mut Option<u64>,
    swap: &mut Option<u64>,
) -> Result<(), zbus::Error> {
    let (cpu_quota, mem_max, swap_max) = if let Some(limit) = cache.get_mut(&id) {
        limit
    } else {
        let proxy = SliceProxy::new(conn, systemd_unit_path(&id)).await?;

        let cpu_quota = proxy.cpu_quota_per_sec_u_sec().await?;
        let mem_max = proxy.memory_max().await?;
        let swap_max = proxy.memory_swap_max().await?;
        cache
            .entry(id)
            .insert_entry((cpu_quota, mem_max, swap_max))
            .into_mut()
    };

    match (cpu, *cpu_quota) {
        (None, u64::MAX) => {}
        (cpu @ None, quota) => *cpu = Some(quota),
        (Some(current), quota) => *current = (*current).min(quota),
    }
    match (mem, *mem_max) {
        (None, u64::MAX) => {}
        (mem @ None, max) => *mem = Some(max),
        (Some(current), max) => *current = (*current).min(max),
    }
    match (swap, *swap_max) {
        (None, u64::MAX) => {}
        (swap @ None, max) => *swap = Some(max),
        (Some(current), max) => *current = (*current).min(max),
    }

    Ok(())
}

async fn usage_time<'a>(
    conn: &zbus::Connection,
    id: CgroupIdentifier<'a>,
) -> Result<(CgroupIdentifier<'a>, u64, Instant), zbus::Error> {
    let proxy = SliceProxy::new(conn, systemd_unit_path(&id)).await?;

    let usage = proxy.cpu_usage_n_sec().await?.saturating_div(1000);

    let time = Instant::now();

    Ok((id, usage, time))
}
