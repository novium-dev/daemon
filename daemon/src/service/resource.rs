// SPDX-License-Identifier: AGPL-3.0-only
use crate::cgroups::cgroup_children;
use crate::{Daemon, DaemonExtra};
use async_trait::async_trait;
use cgroups_rs::fs::Cgroup;
use cgroups_rs::fs::cpuacct::CpuAcctController;
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
use std::{io, thread};
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
                &cgroup_system,
                &kernel_stats_prev,
                &kernel_stats_now,
                usage_system_prev,
                usage_system_now,
                &alloc,
            )?;

            let server = new_resource_subset(
                &cgroup_server,
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
                        cgroup,
                        &kernel_stats_prev,
                        &kernel_stats_now,
                        usage_prev.get(name).copied().unwrap_or_default(),
                        usage_now.get(name).copied().unwrap_or_default(),
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
#[expect(
    clippy::unnecessary_wraps,
    reason = "TODO effective limit needs to walk cgroup tree"
)]
fn new_resource_subset(
    _cgroup: &Cgroup,
    kernel_prev: &KernelStats,
    kernel_new: &KernelStats,
    cpu_prev: u64,
    cpu_new: u64,
    (alloc_cpu, alloc_ram, alloc_swap): &(f64, BigDecimal, BigDecimal),
) -> Result<ResourceSubset, io::Error> {
    // TODO: Effective Limit

    let cpu_delta = cpu_new.saturating_sub(cpu_prev) as f64;
    let system_delta = total_time(kernel_new).saturating_sub(total_time(kernel_prev)) as f64;

    Ok(ResourceSubset {
        cpu_used: ((cpu_delta / system_delta)
            * f64::from(u32::try_from(kernel_new.cpu_time.len()).unwrap_or(1))
            * 10000f64)
            .round()
            / 10000f64,
        cpu_allocated: *alloc_cpu,
        cpu_limit: None,
        mem_allocated: alloc_ram.to_u64().unwrap_or(u64::MAX),
        mem_limit: None,
        mem_used: 0,
        swap_allocated: alloc_swap.to_u64().unwrap_or(u64::MAX),
        swap_limit: None,
        swap_used: 0,
    })
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
