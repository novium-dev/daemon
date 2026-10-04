// SPDX-License-Identifier: AGPL-3.0-only
use crate::cgroup::{HostResources, groups_flat};
use crate::{Daemon, DaemonExtra};
use async_trait::async_trait;
use daemon_common::service::resource::ResourceResponse;
use daemon_common::service::resource::resource_service_error::ResourceInfoError;
use daemon_common::service::resource::{ResourceService, StorageUsage, StorageUsageEntry};
use rustix::fs::{StatVfs, statvfs};
use sqlx::{query, query_scalar};
use std::collections::{HashMap, HashSet};
use std::io;
use tokio::task::JoinSet;
use tracing::error;

#[async_trait]
impl<T: DaemonExtra> ResourceService for Daemon<T> {
    async fn resource_info(&self) -> Result<ResourceResponse, ResourceInfoError> {
        let names: HashSet<String> = query_scalar!("SELECT `name` FROM `groups`")
            .fetch_all(&*self.conn)
            .await
            .inspect_err(|err| error!("Failed to fetch group names: {err:?}"))?
            .into_iter()
            .flat_map(groups_flat)
            .collect();

        let allocation = query!("SELECT `group`, SUM(`cpu`) AS `cpu`, SUM(`ram`) AS `ram`, SUM(`swap`) AS `swap` FROM `servers` GROUP BY `group`")
        .fetch_all(&*self.conn)
        .await
        .inspect_err(|err| error!("Failed to fetch resource allocation: {err:?}"))?
        .into_iter()
        .map(|row| (row.group, (row.cpu.unwrap_or_default(), row.ram.unwrap_or_default(), row.swap.unwrap_or_default())))
        .collect::<HashMap<_, _>>();

        let HostResources {
            cpu_count,
            mem_total,
            swap_total,
            system,
            server,
            groups,
        } = self.host_resources(&names, &allocation).await?;

        let mut storage_server_set = JoinSet::new();
        let mut backup_server_set = JoinSet::new();

        for (id, entry) in &self.config.storage {
            let id = id.clone();
            let path = entry.path().clone();
            if !tokio::fs::try_exists(&path).await.inspect_err(|err| {
                error!("Server storage {id:?} stat failed (try_exists): {err:?}");
            })? {
                tokio::fs::create_dir_all(&path).await.inspect_err(|err| {
                    error!("Server storage {id:?} stat failed (create_dir_all): {err:?}");
                })?;
            }
            storage_server_set.spawn_blocking(move || {
                let stat = statvfs(&path).inspect_err(|err| {
                    error!("Server storage {id:?} stat failed (statvfs): {err:?}");
                })?;
                Ok::<_, io::Error>((id, stat_to_storage_usage(path, &stat)))
            });
        }

        for (id, entry) in &self.config.backup {
            let id = id.clone();
            let path = entry.path().clone();
            if !tokio::fs::try_exists(&path).await.inspect_err(|err| {
                error!("Backup storage {id:?} stat failed (try_exists): {err:?}");
            })? {
                tokio::fs::create_dir_all(&path).await.inspect_err(|err| {
                    error!("Backup storage {id:?} stat failed (create_dir_all): {err:?}");
                })?;
            }
            backup_server_set.spawn_blocking(move || {
                let stat = statvfs(&path).inspect_err(|err| {
                    error!("Backup storage {id:?} stat failed (statvfs): {err:?}");
                })?;
                Ok::<_, io::Error>((id, stat_to_storage_usage(path, &stat)))
            });
        }

        Ok(ResourceResponse {
            cpu_count,
            mem_total,
            swap_total,
            system,
            server,
            groups,
            storage: StorageUsage {
                backup: backup_server_set
                    .join_all()
                    .await
                    .into_iter()
                    .collect::<Result<_, _>>()?,
                server: storage_server_set
                    .join_all()
                    .await
                    .into_iter()
                    .collect::<Result<_, _>>()?,
            },
        })
    }
}

const fn stat_to_storage_usage(path: String, stat: &StatVfs) -> StorageUsageEntry {
    StorageUsageEntry {
        path,
        total: stat.f_frsize.saturating_mul(stat.f_blocks) >> 20u64,
        free: stat.f_frsize.saturating_mul(stat.f_bfree) >> 20u64,
        available: stat.f_frsize.saturating_mul(stat.f_bavail) >> 20u64,
    }
}
