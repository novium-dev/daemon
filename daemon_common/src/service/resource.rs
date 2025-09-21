// SPDX-License-Identifier: AGPL-3.0-only
use super::*;
use serde::{Deserialize, Serialize};
use zelus::service;

#[service(tag = "resource")]
pub trait ResourceService {
    /// Gets resource information
    #[route("/resource", method = GET)]
    #[error(sql(failure [sql:*]) { Resource "Unable to retrieve the resource info" INTERNAL_SERVER_ERROR })]
    async fn resource_info(&self) -> Result<ResourceResponse, _>;
}

#[derive(Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
#[schema(example = json!({
  "mem_total": 67_319_472_128u64,
  "mem_used": 6_551_359_488u64,
  "mem_allocated": 0u64,
  "swap_total": 137_438_949_376u64,
  "swap_used": 0u64,
  "swap_allocated": 0u64,
  "cpu_used": 1.198_044_2f32,
  "cpu_count": 12u32,
  "disks": [
    {
      "available": 392_462_217_216u64,
      "total": 1_966_021_279_744u64,
      "mountpoint": "/",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392_462_217_216u64,
      "total": 1_966_021_279_744u64,
      "mountpoint": "/boot",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392_462_217_216u64,
      "total": 1_966_021_279_744u64,
      "mountpoint": "/root",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392_462_217_216u64,
      "total": 1_966_021_279_744u64,
      "mountpoint": "/home",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392_462_217_216u64,
      "total": 1_966_021_279_744u64,
      "mountpoint": "/srv",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392_462_217_216u64,
      "total": 1_966_021_279_744u64,
      "mountpoint": "/var/cache",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392_462_217_216u64,
      "total": 1_966_021_279_744u64,
      "mountpoint": "/var/log",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392_462_217_216u64,
      "total": 1_966_021_279_744u64,
      "mountpoint": "/var/swap",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392_462_217_216u64,
      "total": 1_966_021_279_744u64,
      "mountpoint": "/var/tmp",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 34_058_813_440u64,
      "total": 34_342_944_768u64,
      "mountpoint": "/efi",
      "readonly": false,
      "filesystem": "vfat"
    },
    {
      "available": 392_462_217_216u64,
      "total": 1_966_021_279_744u64,
      "mountpoint": "/var/lib/docker/btrfs",
      "readonly": false,
      "filesystem": "btrfs"
    }
  ]
}))]
pub struct ResourceResponse {
    pub mem_total: u64,
    pub mem_used: u64,
    pub mem_allocated: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub swap_allocated: u64,
    pub cpu_used: f32,
    pub cpu_count: u32,
    pub disks: Vec<ResourceDisk>,
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Hash, utoipa::ToSchema)]
pub struct ResourceDisk {
    pub available: u64,
    pub total: u64,
    pub mountpoint: String,
    pub readonly: bool,
    pub filesystem: String,
}
