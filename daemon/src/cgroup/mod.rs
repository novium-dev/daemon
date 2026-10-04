// SPDX-License-Identifier: AGPL-3.0-only
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
pub(crate) use linux::cgroup_parent;
#[cfg(target_os = "macos")]
pub(crate) use macos::cgroup_parent;

use std::{borrow::Cow, collections::HashMap};

use daemon_common::service::resource::ResourceSubset;
use num_traits::FromPrimitive as _;
use sqlx::types::BigDecimal;

pub(crate) const DEFAULT_CPU_PERIOD: u64 = 100_000u64;
const USEC: u64 = 1_000_000u64;
const USEC_F: f64 = 1_000_000f64;

pub(crate) const GROUP_LABEL: &str = "world.novium.group";

pub(crate) type Allocation = HashMap<Option<String>, (f64, BigDecimal, BigDecimal)>;

pub(crate) struct HostResources {
    pub cpu_count: u32,
    pub mem_total: u64,
    pub swap_total: u64,
    pub system: ResourceSubset,
    pub server: ResourceSubset,
    pub groups: HashMap<String, ResourceSubset>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum CgroupIdentifier<'a> {
    Group(Cow<'a, str>),
    #[cfg_attr(
        target_os = "macos",
        expect(dead_code, reason = "macOS has no host cgroup to report")
    )]
    System,
    Server,
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
