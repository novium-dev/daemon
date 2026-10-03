use std::collections::HashMap;

// SPDX-License-Identifier: AGPL-3.0-only
use super::*;
use serde::{Deserialize, Serialize};
use zelus::service;

#[service(tag = "resource")]
pub trait ResourceService {
    /// Gets resource information
    #[route("/resource", method = GET)]
    #[error(sql::failure <- [sql:*], (Resource, "Unable to retrieve the resource info", INTERNAL_SERVER_ERROR) <- [io:*])]
    async fn resource_info(&self) -> Result<ResourceResponse, _>;
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct ResourceResponse {
    pub mem_total: u64,
    pub swap_total: u64,
    pub cpu_count: u32,
    pub system: ResourceSubset,
    pub server: ResourceSubset,
    pub groups: HashMap<String, ResourceSubset>,
    pub storage: StorageUsage,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct ResourceSubset {
    pub mem_limit: Option<u64>,
    pub mem_limit_effective: Option<u64>,
    pub mem_used: u64,
    pub mem_allocated: u64,
    pub swap_limit: Option<u64>,
    pub swap_limit_effective: Option<u64>,
    pub swap_used: u64,
    pub swap_allocated: u64,
    pub cpu_limit: Option<f64>,
    pub cpu_limit_effective: Option<f64>,
    pub cpu_used: f64,
    pub cpu_allocated: f64,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct StorageUsage {
    pub server: HashMap<String, StorageUsageEntry>,
    pub backup: HashMap<String, StorageUsageEntry>,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct StorageUsageEntry {
    pub path: String,
    pub free: u64,
    pub available: u64,
    pub total: u64,
}
