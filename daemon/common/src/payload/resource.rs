use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
#[schema(example = json!({
  "mem_total": 67319472128u64,
  "mem_used": 6551359488u64,
  "mem_allocated": 0u64,
  "swap_total": 137438949376u64,
  "swap_used": 0u64,
  "swap_allocated": 0u64,
  "cpu_used": 1.1980442f32,
  "cpu_count": 12u32,
  "disks": [
    {
      "available": 392462217216u64,
      "total": 1966021279744u64,
      "mountpoint": "/",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392462217216u64,
      "total": 1966021279744u64,
      "mountpoint": "/boot",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392462217216u64,
      "total": 1966021279744u64,
      "mountpoint": "/root",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392462217216u64,
      "total": 1966021279744u64,
      "mountpoint": "/home",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392462217216u64,
      "total": 1966021279744u64,
      "mountpoint": "/srv",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392462217216u64,
      "total": 1966021279744u64,
      "mountpoint": "/var/cache",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392462217216u64,
      "total": 1966021279744u64,
      "mountpoint": "/var/log",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392462217216u64,
      "total": 1966021279744u64,
      "mountpoint": "/var/swap",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 392462217216u64,
      "total": 1966021279744u64,
      "mountpoint": "/var/tmp",
      "readonly": false,
      "filesystem": "btrfs"
    },
    {
      "available": 34058813440u64,
      "total": 34342944768u64,
      "mountpoint": "/efi",
      "readonly": false,
      "filesystem": "vfat"
    },
    {
      "available": 392462217216u64,
      "total": 1966021279744u64,
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
