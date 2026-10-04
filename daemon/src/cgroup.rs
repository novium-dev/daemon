// SPDX-License-Identifier: AGPL-3.0-only
use std::{borrow::Cow, collections::HashMap, io};

use num_traits::FromPrimitive as _;
use sqlx::query;
use tracing::error;
use zbus_systemd::systemd1::UnitProxy;

use crate::{Daemon, DaemonExtra, config::LimitConfig, utils::from_mib};

const DEFAULT_CPU_PERIOD: u64 = 100_000u64;
const USEC: u64 = 1_000_000u64;
const USEC_F: f64 = 1_000_000f64;

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum CgroupIdentifier<'a> {
    Group(Cow<'a, str>),
    System,
    Server,
}

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

        if cfg!(not(target_os = "linux")) {
            return Ok(());
        }

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
}

pub(crate) fn clamp_cpu_limit<const BASE_USEC: bool>(cpu: f32) -> Option<(u64, u64)> {
    let quota = u64::from_f64((f64::from(cpu.abs()) * USEC_F).round())?.max(1000);
    let period = if quota < 10000u64 {
        USEC
    } else {
        DEFAULT_CPU_PERIOD
    };
    if BASE_USEC {
        Some((quota, period))
    } else {
        Some(((quota.saturating_mul(period)).div_ceil(USEC), period))
    }
}

#[expect(clippy::indexing_slicing, reason = "there is no better variant yet")]
pub(crate) fn groups_flat(name: String) -> Vec<String> {
    if name.matches('-').count() == 0 {
        return vec![name];
    }
    let mut components: Vec<_> = name.split('-').map(String::from).collect();

    for i in 1..components.len() {
        components[i] = format!("{}-{}", components[i.saturating_sub(1)], components[i]);
    }
    components
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

#[cfg(test)]
mod test {
    #[test]
    fn clamp_cpu_limit() {
        use super::*;

        assert_eq!(
            clamp_cpu_limit::<true>(1.0),
            Some((USEC, DEFAULT_CPU_PERIOD))
        );
        assert_eq!(
            clamp_cpu_limit::<false>(1.0),
            Some((DEFAULT_CPU_PERIOD, DEFAULT_CPU_PERIOD))
        );
        assert_eq!(
            clamp_cpu_limit::<true>(10000.0),
            Some((10_000_000_000, DEFAULT_CPU_PERIOD))
        );
        assert_eq!(
            clamp_cpu_limit::<false>(10000.0),
            Some((1_000_000_000, DEFAULT_CPU_PERIOD))
        );
        assert_eq!(
            clamp_cpu_limit::<true>(0.01),
            Some((10000, DEFAULT_CPU_PERIOD))
        );
        assert_eq!(
            clamp_cpu_limit::<false>(0.01),
            Some((1000, DEFAULT_CPU_PERIOD))
        );
        assert_eq!(
            clamp_cpu_limit::<false>(0.01),
            Some((1000, DEFAULT_CPU_PERIOD))
        );
        assert_eq!(
            clamp_cpu_limit::<false>(-0.01),
            Some((1000, DEFAULT_CPU_PERIOD))
        );
        assert_eq!(clamp_cpu_limit::<false>(0.001), Some((1000, USEC)));
        assert_eq!(clamp_cpu_limit::<true>(0.001), Some((1000, USEC)));
        assert_eq!(clamp_cpu_limit::<false>(0.005), Some((5000, USEC)));
        assert_eq!(clamp_cpu_limit::<true>(0.005), Some((5000, USEC)));
        assert_eq!(clamp_cpu_limit::<true>(0.005_432), Some((5432, USEC)));
        assert_eq!(clamp_cpu_limit::<false>(0.000_001), Some((1000, USEC)));
        assert_eq!(clamp_cpu_limit::<true>(0.000_001), Some((1000, USEC)));
        assert_eq!(clamp_cpu_limit::<true>(f32::MAX), None);
        assert_eq!(clamp_cpu_limit::<true>(f32::INFINITY), None);
        assert_eq!(clamp_cpu_limit::<true>(f32::NAN), None);
    }
}
