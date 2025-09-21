// SPDX-License-Identifier: AGPL-3.0-only
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Port {
    pub internal: u16,
    pub external: u16,
    pub external_ip: String,
    pub protocol: String,
}

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub struct AssignedNetwork {
    pub network_id: u32,
    pub network_ipv4: u16,
    pub network_ipv6: u16,
    pub server_ipv4: u8,
    pub server_ipv6: u64,
}
