#[derive(Debug, Clone, sqlx::FromRow)]
pub(crate) struct Port {
    pub(crate) internal_port: u16,
    pub(crate) external_port: u16,
    pub(crate) external_ip: String,
    pub(crate) protocol: String,
}

#[derive(Debug, Clone, Default, sqlx::FromRow)]
pub(crate) struct AssignedNetwork {
    pub(crate) network_id: u32,
    pub(crate) network_ipv4: u16,
    pub(crate) network_ipv6: u16,
    pub(crate) server_ipv4: u8,
    pub(crate) server_ipv6: u64,
}
