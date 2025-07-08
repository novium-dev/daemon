/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
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
