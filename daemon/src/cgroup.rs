// SPDX-License-Identifier: AGPL-3.0-only
use std::{borrow::Cow, collections::HashMap, io};

use sqlx::query;
use tracing::error;
use zbus_systemd::systemd1::UnitProxy;

use crate::{Daemon, DaemonExtra, config::LimitConfig, utils::from_mib};

pub const CPU_PERIOD: u64 = 100_000u64;
pub const CPU_PERIOD_F: f64 = 100_000f64;

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

        for (name, limit) in groups {
            self.cgroup_apply_limit::<E>(CgroupIdentifier::Group(Cow::Owned(name)), limit)
                .await?;
        }

        Ok(())
    }

    #[expect(clippy::cast_sign_loss, reason = "not possible")]
    async fn cgroup_apply_limit<E: From<zbus::Error>>(
        &self,
        id: CgroupIdentifier<'_>,
        limit: LimitConfig,
    ) -> Result<(), E> {
        let unit = UnitProxy::new(&self.dbus, systemd_unit_path(id)).await?;

        let cpu_quota = limit.cpu.map_or(u64::MAX, |cpu| {
            (f64::from(cpu.abs()) * f64::from(1_000_000i32)).round() as u64
        });

        let memory_max = limit.ram.map_or(u64::MAX, from_mib);
        let swap_max = limit.swap.map_or(u64::MAX, from_mib);

        unit.set_properties(
            true,
            vec![
                ("CPUQuotaPeriodUSec".to_owned(), CPU_PERIOD.into()),
                ("CPUQuotaPerSecUSec".to_owned(), cpu_quota.into()),
                ("MemoryMax".to_owned(), memory_max.into()),
                ("MemorySwapMax".to_owned(), swap_max.into()),
            ],
        )
        .await?;

        Ok(())
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
pub(crate) fn systemd_unit_path(id: CgroupIdentifier<'_>) -> String {
    format!(
        "/org/freedesktop/systemd1/unit/{}",
        systemd_slice_handle(id)
            .replace('_', "_5f")
            .replace('-', "_2d")
            .replace('.', "_2e")
    )
}

#[inline]
pub(crate) fn systemd_slice_handle(group: CgroupIdentifier<'_>) -> String {
    match group {
        CgroupIdentifier::Group(name) => {
            format!("{}-{name}.slice", crate::common::DAEMON_CGROUP_ROOT)
        }
        CgroupIdentifier::Server => format!("{}.slice", crate::common::DAEMON_CGROUP_ROOT),
        CgroupIdentifier::System => "-.slice".to_owned(),
    }
}
