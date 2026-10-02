// SPDX-License-Identifier: AGPL-3.0-only
use crate::cgroups::{cgroup_absolute_path, cgroup_children};
use crate::{Daemon, DaemonExtra};
use async_trait::async_trait;
use cgroups_rs::fs::cpu::CpuController;
use cgroups_rs::fs::cpuacct::CpuAcctController;
use cgroups_rs::fs::memory::MemController;
use cgroups_rs::fs::{Cgroup, MaxValue, parse_max_value};
use core::time::Duration;
use daemon_common::service::resource::ResourceService;
use daemon_common::service::resource::resource_service_error::ResourceInfoError;
use daemon_common::service::resource::{ResourceResponse, ResourceSubset};
use num_traits::ToPrimitive as _;
use procfs::{CpuInfo, Current as _, CurrentSI as _, KernelStats, Meminfo};
use sqlx::query;
use sqlx::types::BigDecimal;
use std::collections::HashMap;
use std::sync::Arc;
use std::{fs, io, thread};
use tracing::error;

#[async_trait]
impl<T: DaemonExtra> ResourceService for Daemon<T> {
    async fn resource_info(&self) -> Result<ResourceResponse, ResourceInfoError> {
        let allocation = query!("SELECT `group`, SUM(`cpu`) AS `cpu`, SUM(`ram`) AS `ram`, SUM(`swap`) AS `swap` FROM `servers` GROUP BY `group`")
        .fetch_all(&*self.conn)
        .await
        .inspect_err(|err| error!("Failed to retrieve memory usage from servers: {:?}", err))?
        .into_iter()
        .filter_map(|row| row.group.map(|group| (group, (row.cpu.unwrap_or_default(), row.ram.unwrap_or_default(), row.swap.unwrap_or_default()))))
        .collect::<HashMap<_, _>>();

        let cgroup_system = Arc::clone(&self.cgroup_system);
        let cgroup_server = Arc::clone(&self.cgroup_server);
        let cgroups = cgroup_children(&cgroup_server)
            .await
            .map_err(|_err| ResourceInfoError::Resource)?;

        tokio::task::spawn_blocking(move || {
            let mut usage_prev = HashMap::with_capacity(cgroups.len());
            let mut usage_now = HashMap::with_capacity(cgroups.len());

            // fast path start
            let kernel_stats_prev = KernelStats::current()?;
            let usage_system_prev = cgroup_system
                .controller_of::<CpuAcctController>()
                .ok_or_else(|| io::Error::other("CPU Account Controller not available"))?
                .cpuacct()
                .usage;
            let usage_server_prev = cgroup_system
                .controller_of::<CpuAcctController>()
                .ok_or_else(|| io::Error::other("CPU Account Controller not available"))?
                .cpuacct()
                .usage;
            for (name, cgroup) in &cgroups {
                usage_prev.insert(
                    name,
                    cgroup
                        .controller_of::<CpuAcctController>()
                        .ok_or_else(|| io::Error::other("CPU Account Controller not available"))?
                        .cpuacct()
                        .usage,
                );
            }
            // fast path end

            thread::sleep(Duration::from_millis(200)); // Recommended time by sysinfo

            // fast path start
            let kernel_stats_now = KernelStats::current()?;
            let usage_system_now = cgroup_system
                .controller_of::<CpuAcctController>()
                .ok_or_else(|| io::Error::other("CPU Account Controller not available"))?
                .cpuacct()
                .usage;
            let usage_server_now = cgroup_system
                .controller_of::<CpuAcctController>()
                .ok_or_else(|| io::Error::other("CPU Account Controller not available"))?
                .cpuacct()
                .usage;
            for (name, cgroup) in &cgroups {
                usage_now.insert(
                    name,
                    cgroup
                        .controller_of::<CpuAcctController>()
                        .ok_or_else(|| io::Error::other("CPU Account Controller not available"))?
                        .cpuacct()
                        .usage,
                );
            }
            // fast path end

            let cpu = CpuInfo::current()?;
            let mem = Meminfo::current()?;

            let mut groups = HashMap::with_capacity(cgroups.len());

            let cpu_alloc = allocation.values().map(|alloc| alloc.0).sum();
            let ram_alloc: BigDecimal = allocation.values().map(|alloc| alloc.1.clone()).sum();
            let swap_alloc: BigDecimal = allocation.values().map(|alloc| alloc.2.clone()).sum();
            let alloc = (cpu_alloc, ram_alloc, swap_alloc);

            let system = new_resource_subset(
                (&cgroup_system, &cgroup_server),
                Some(&cgroup_system),
                &kernel_stats_prev,
                &kernel_stats_now,
                usage_system_prev,
                usage_system_now,
                &alloc,
            )?;

            let server = new_resource_subset(
                (&cgroup_system, &cgroup_server),
                Some(&cgroup_server),
                &kernel_stats_prev,
                &kernel_stats_now,
                usage_server_prev,
                usage_server_now,
                &alloc,
            )?;

            for (name, cgroup) in &cgroups {
                let alloc = allocation.get(name).cloned().unwrap_or_default();
                groups.insert(
                    name.clone(),
                    new_resource_subset(
                        (&cgroup_system, &cgroup_server),
                        Some(cgroup),
                        &kernel_stats_prev,
                        &kernel_stats_now,
                        usage_prev.get(name).copied().unwrap_or_default(),
                        usage_now.get(name).copied().unwrap_or_default(),
                        &alloc,
                    )?,
                );
            }

            for (name, alloc) in allocation {
                if groups.contains_key(&name) {
                    continue;
                }
                groups.insert(
                    name,
                    new_resource_subset(
                        (&cgroup_system, &cgroup_server),
                        None,
                        &kernel_stats_prev,
                        &kernel_stats_now,
                        0,
                        0,
                        &alloc,
                    )?,
                );
            }

            Ok::<_, anyhow::Error>(ResourceResponse {
                cpu_count: u32::try_from(cpu.num_cores()).unwrap_or_default(),
                mem_total: mem.mem_total,
                swap_total: mem.swap_total,
                system,
                server,
                groups,
                backups: HashMap::new(),  // TODO
                storages: HashMap::new(), // TODO
            })
        })
        .await
        .inspect_err(|err| error!("Failed to provide resource info (task): {err:?}"))
        .map_err(|_err| ResourceInfoError::Resource)?
        .inspect_err(|err| error!("Failed to provide resource info: {err:?}"))
        .map_err(|_err| ResourceInfoError::Resource)
    }
}

#[expect(clippy::cast_precision_loss, reason = "cpu doesn't have to be precise")]
fn new_resource_subset(
    (cgroup_system, cgroup_server): (&Cgroup, &Cgroup),
    cgroup: Option<&Cgroup>,
    kernel_prev: &KernelStats,
    kernel_new: &KernelStats,
    cpu_prev: u64,
    cpu_new: u64,
    (alloc_cpu, alloc_ram, alloc_swap): &(f64, BigDecimal, BigDecimal),
) -> Result<ResourceSubset, io::Error> {
    let cpu_delta = cpu_new.saturating_sub(cpu_prev) as f64;
    let system_delta = total_time(kernel_new).saturating_sub(total_time(kernel_prev)) as f64;

    let mut cpu_limit = None;
    let mut mem_limit = None;
    let mut swap_limit = None;

    apply_limit(
        cgroup_system,
        &mut cpu_limit,
        &mut mem_limit,
        &mut swap_limit,
    )?;
    apply_limit(
        cgroup_server,
        &mut cpu_limit,
        &mut mem_limit,
        &mut swap_limit,
    )?;

    let (mem_used, swap_used) = if let Some(cgroup) = cgroup {
        apply_limit(cgroup, &mut cpu_limit, &mut mem_limit, &mut swap_limit)?;
        let stat = cgroup
            .controller_of::<MemController>()
            .ok_or_else(|| io::Error::other("Memory Controller not available"))?
            .memory_stat();
        (stat.usage_in_bytes, stat.swappiness)
    } else {
        (0, 0)
    };

    Ok(ResourceSubset {
        cpu_used: ((cpu_delta / system_delta)
            * f64::from(u32::try_from(kernel_new.cpu_time.len()).unwrap_or(1))
            * 10000f64)
            .round()
            / 10000f64,
        cpu_allocated: *alloc_cpu,
        cpu_limit,
        mem_allocated: alloc_ram.to_u64().unwrap_or(u64::MAX),
        mem_limit: mem_limit.map(|limit| limit >> 20u32),
        mem_used,
        swap_allocated: alloc_swap.to_u64().unwrap_or(u64::MAX),
        swap_limit: swap_limit.map(|limit| limit >> 20u32),
        swap_used,
    })
}

#[expect(clippy::cast_precision_loss, reason = "cpu doesn't have to be precise")]
#[inline]
fn apply_limit(
    cgroup: &Cgroup,
    cpu: &mut Option<f64>,
    ram: &mut Option<u64>,
    swap: &mut Option<u64>,
) -> Result<(), io::Error> {
    if let Some(controller) = cgroup.controller_of::<CpuController>() {
        let period = controller.cfs_period().map_err(io::Error::other)?;
        let quota = controller.cfs_quota().map_err(io::Error::other)?;

        if quota != -1 {
            let limit = quota as f64 / period as f64;
            if let Some(value) = cpu {
                if *value > limit {
                    *value = limit;
                }
            } else {
                *cpu = Some(limit);
            }
        }
    }
    if let Some(controller) = cgroup.controller_of::<MemController>() {
        if let MaxValue::Value(max_mem) = controller
            .get_mem()
            .map_err(io::Error::other)?
            .max
            .unwrap_or(MaxValue::Max)
            && max_mem >= 0
            && ram.is_none_or(|ram| ram > max_mem.unsigned_abs())
        {
            *ram = Some(max_mem.unsigned_abs());
        }
        // For some reason, controller.memswap_v2 creates a parse error if swap is unlimited, resulting in a error message and return value of 0.
        let memory_swap_max =
            fs::read_to_string(cgroup_absolute_path(cgroup).join("memory.swap.max"))
                .inspect_err(|err| error!("failed to read swap limit: {err:?}"))?;
        if let MaxValue::Value(max_swap) = parse_max_value(&memory_swap_max)
            .inspect_err(|err| error!("failed to parse swap limit: {err:?}"))
            .map_err(io::Error::other)?
            && max_swap >= 0
            && swap.is_none_or(|swap| swap > max_swap.unsigned_abs())
        {
            *swap = Some(max_swap.unsigned_abs());
        }
    }

    Ok(())
}

fn total_time(kernel: &KernelStats) -> u64 {
    kernel
        .total
        .user
        .saturating_add(kernel.total.nice)
        .saturating_add(kernel.total.system)
        .saturating_add(kernel.total.irq.unwrap_or_default())
        .saturating_add(kernel.total.softirq.unwrap_or_default())
        .saturating_add(kernel.total.idle)
        .saturating_add(kernel.total.iowait.unwrap_or_default())
        .saturating_add(kernel.total.guest.unwrap_or_default())
        .saturating_add(kernel.total.guest_nice.unwrap_or_default())
        .saturating_add(kernel.total.steal.unwrap_or_default())
}
