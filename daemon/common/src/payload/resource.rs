use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ResourceResponse {
    pub mem_total: u64,
    pub mem_used: u64,
    pub mem_allocated: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub swap_allocated: u64,
    pub cpu_used: f32,
    pub cpu_count: u64,
    pub disks: Vec<ResourceDisk>,
}

#[derive(Serialize, Deserialize)]
pub struct ResourceDisk {
    pub available: u64,
    pub total: u64,
    pub mountpoint: String,
    pub readonly: bool,
    pub filesystem: String,
}
