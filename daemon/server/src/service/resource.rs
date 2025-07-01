use crate::Daemon;
use async_trait::async_trait;
use daemon_common::service::resource::ResourceService;
use daemon_common::service::resource::error::ResourceInfoError;
use daemon_common::service::resource::{ResourceDisk, ResourceResponse};
use num_traits::ToPrimitive as _;
use sqlx::query_as;
use sqlx::types::BigDecimal;
use std::thread;
use core::time::Duration;
use tracing::{debug, error};

#[async_trait]
impl ResourceService for Daemon {
    async fn resource_info(&self) -> Result<ResourceResponse, ResourceInfoError> {
        #[derive(sqlx::FromRow)]
        struct ResourceSummary {
            ram: Option<BigDecimal>,
            swap: Option<BigDecimal>,
        }

        let ResourceSummary {
            ram: mem_allocated,
            swap: swap_allocated,
        } = query_as!(
            ResourceSummary,
            "SELECT SUM(`ram`) AS `ram`, SUM(`swap`) AS `swap` FROM `servers`"
        )
        .fetch_one(&*self.conn)
        .await
        .inspect_err(|err| error!("Failed to retrieve memory usage from servers: {:?}", err))?;

        tokio::task::spawn_blocking(move || {
            let mut system = sysinfo::System::new_all();
            thread::sleep(Duration::from_millis(200)); // Recommended time by sysinfo
            system.refresh_cpu_all();

            let disks = sysinfo::Disks::new_with_refreshed_list();

            ResourceResponse {
                mem_total: system.total_memory(),
                mem_used: system.used_memory(),
                mem_allocated: mem_allocated
                    .unwrap_or_default()
                    .to_u64()
                    .unwrap_or_default()
                    .saturating_mul(0x0010_0000),
                swap_total: system.total_swap(),
                swap_used: system.used_swap(),
                swap_allocated: swap_allocated
                    .unwrap_or_default()
                    .to_u64()
                    .unwrap_or_default()
                    .saturating_mul(0x0010_0000),
                cpu_count: system.cpus().len() as u32,
                cpu_used: system
                    .cpus()
                    .iter()
                    .map(|item| item.cpu_usage() / 100.0)
                    .sum(),
                disks: disks
                    .into_iter()
                    .map(|disk| ResourceDisk {
                        total: disk.total_space(),
                        available: disk.available_space(),
                        mountpoint: disk.mount_point().to_string_lossy().to_string(),
                        readonly: disk.is_read_only(),
                        filesystem: disk.file_system().to_string_lossy().to_string(),
                    })
                    .collect(),
            }
        })
        .await
        .inspect_err(|err| debug!("Failed to provide resource info: {:?}", err))
        .map_err(|_err| ResourceInfoError::ResourceError)
    }
}
