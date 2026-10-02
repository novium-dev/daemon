// SPDX-License-Identifier: AGPL-3.0-only
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::PathBuf,
};

use cgroups_rs::fs::{
    Cgroup, ControllIdentifier as _, Hierarchy as _, cpu::CpuController,
    cpuacct::CpuAcctController, hierarchies::V2, memory::MemController,
};
use num_traits::FromPrimitive as _;
use sqlx::query;
use tokio_stream::{StreamExt as _, wrappers::ReadDirStream};
use tracing::{error, warn};

use crate::{
    Daemon, DaemonExtra,
    config::{DaemonConfig, LimitConfig},
    init::DaemonInitError,
    utils::from_mib,
};

pub const CPU_PERIOD: i64 = 100_000i64;
pub const CPU_PERIOD_F: f64 = 100_000f64;

pub async fn init(config: &DaemonConfig) -> Result<(Cgroup, Cgroup), DaemonInitError> {
    let limit = config.limit;
    tokio::task::spawn_blocking(move || {
        Ok((
            V2::new().root_control_group(),
            init_cgroup_blocking(crate::common::DAEMON_CGROUP_ROOT, limit)?,
        ))
    })
    .await
    .map_err(DaemonInitError::Join)?
}

impl<T: DaemonExtra> Daemon<T> {
    /// Cancel safe, kinda.
    pub async fn sync_cgroups<E: From<sqlx::Error> + From<io::Error>>(&self) -> Result<(), E> {
        let _guard = self.cgroup_lock.lock().await;

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

        let current = cgroup_children_names(&self.cgroup_server).await?;

        for name in current
            .into_iter()
            .filter(|name| !groups.contains_key(name))
        {
            let path = format!("{}/{name}", crate::common::DAEMON_CGROUP_ROOT); // Validated by sql constraint
            tokio::task::spawn_blocking(move || {
                let cgroup = Cgroup::load_with_specified_controllers(
                    Box::new(V2::new()),
                    path,
                    controllers(),
                );

                let pids = cgroup.procs();

                if pids.is_empty() {
                    cgroup
                        .delete()
                        .inspect_err(|err| error!("Failed to delete cgroup: {err:?}"))
                } else {
                    warn!("Not deleting cgroup {name}. Pids: {pids:?}");
                    Ok(())
                }
            })
            .await
            .map_err(io::Error::from)?
            .map_err(io::Error::other)?;
        }

        for (name, limit) in groups {
            let path = format!("{}/{name}", crate::common::DAEMON_CGROUP_ROOT); // Validated by sql constraint
            tokio::task::spawn_blocking(move || init_cgroup_blocking(&path, limit))
                .await
                .map_err(io::Error::from)?
                .map_err(io::Error::other)?;
        }

        Ok(())
    }
}

fn init_cgroup_blocking(path: &str, limit: LimitConfig) -> Result<Cgroup, DaemonInitError> {
    let cgroup = Cgroup::load_with_specified_controllers(Box::new(V2::new()), path, controllers());

    if !cgroup_exists_blocking(&cgroup).map_err(DaemonInitError::Io)? {
        cgroup
            .create()
            .inspect_err(|err| error!("Failed to create cgroup: {err:?}"))
            .map_err(DaemonInitError::Cgroup)?;
    }

    let cpu_controller = cgroup_controller_of::<CpuController>(&cgroup)?;
    cpu_controller
        .set_cfs_period(CPU_PERIOD.unsigned_abs())
        .inspect_err(|err| error!("Failed to set cgroup cpu period: {err:?}"))
        .map_err(DaemonInitError::Cgroup)?;

    cpu_controller
        .set_cfs_quota(if let Some(cpu) = limit.cpu {
            i64::from_f64((f64::from(cpu) * CPU_PERIOD_F).round()).ok_or(DaemonInitError::Math)?
        } else {
            -1
        })
        .inspect_err(|err| error!("Failed to set cgroup cpu quota: {err:?}"))
        .map_err(DaemonInitError::Cgroup)?;

    let mem_controller = cgroup_controller_of::<MemController>(&cgroup)?;
    let memory = limit
        .ram
        .map(from_mib)
        .map(|ram| i64::try_from(ram).unwrap_or(i64::MAX));

    mem_controller
        .set_limit(memory.unwrap_or(-1))
        .inspect_err(|err| error!("Failed to set cgroup memory limit: {err:?}"))
        .map_err(DaemonInitError::Cgroup)?;

    mem_controller
        .set_memswap_limit(
            limit
                .swap
                .map(from_mib)
                .map_or(-1, |swap| i64::try_from(swap).unwrap_or(-1)),
        )
        .inspect_err(|err| error!("Failed to set cgroup memory swap limit: {err:?}"))
        .map_err(DaemonInitError::Cgroup)?;

    Ok(cgroup)
}

#[inline]
fn cgroup_exists_blocking(cgroup: &Cgroup) -> Result<bool, io::Error> {
    fs::exists(cgroup_absolute_path(cgroup))
        .inspect_err(|err| error!("Failed to check if cgroup does exist: {err:?}"))
}

pub async fn cgroup_children(cgroup: &Cgroup) -> Result<HashMap<String, Cgroup>, io::Error> {
    let names = cgroup_children_names(cgroup).await?;
    tokio::task::spawn_blocking(move || {
        names
            .into_iter()
            .map(|name| {
                let path = format!("{}/{name}", crate::common::DAEMON_CGROUP_ROOT);
                (
                    name,
                    Cgroup::load_with_specified_controllers(
                        Box::new(V2::new()),
                        path,
                        controllers(),
                    ),
                )
            })
            .collect()
    })
    .await
    .map_err(io::Error::from)
}

async fn cgroup_children_names(cgroup: &Cgroup) -> Result<HashSet<String>, io::Error> {
    let readdir = tokio::fs::read_dir(cgroup_absolute_path(cgroup))
        .await
        .inspect_err(|err| error!("Failed to iterate children of cgroup: {err:?}"))?;
    ReadDirStream::new(readdir)
        .map(|entry| {
            entry?
                .file_name()
                .into_string()
                .map_err(|str| io::Error::other(format!("Invalid cgroup found: {}", str.display())))
        })
        .collect()
        .await
}

#[inline]
pub(crate) fn cgroup_absolute_path(cgroup: &Cgroup) -> PathBuf {
    cgroup_absolute_path_by_name(cgroup.path())
}

#[inline]
pub(crate) fn cgroup_absolute_path_by_name(name: &str) -> PathBuf {
    V2::new().root().join(name)
}

#[inline]
fn cgroup_controller_of<'a, T>(cgroup: &'a Cgroup) -> Result<&'a T, DaemonInitError>
where
    &'a T: From<&'a cgroups_rs::fs::Subsystem>,
    T: cgroups_rs::fs::Controller + cgroups_rs::fs::ControllIdentifier,
{
    cgroup
        .controller_of::<T>()
        .ok_or(DaemonInitError::CgroupControllerMissing(
            T::controller_type(),
        ))
}

pub fn controllers() -> Vec<String> {
    vec![
        CpuController::controller_type().to_string(),
        CpuAcctController::controller_type().to_string(),
    ]
}
