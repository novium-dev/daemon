// SPDX-License-Identifier: AGPL-3.0-only
pub const LABEL_NAME: &str = "cloud.novium.daemon.server";
pub const IMAGE_NAME: &str = "novium-daemon-image";
pub const CONTAINER_PREFIX: &str = "novium-daemon_";

// Should always be the same as the euid and egid of the daemon process
#[cfg(not(target_os = "macos"))]
pub const CONTAINER_DEFAULT_UID: u16 = if cfg!(debug_assertions) { 1000 } else { 25335 };

#[cfg(target_os = "macos")]
pub const CONTAINER_DEFAULT_UID: u16 = 501;

pub const CONTAINER_DEFAULT_GID: u16 = CONTAINER_DEFAULT_UID;

pub const MARIADB_PLUGIN_NAME: &str = "novium_daemon_auth";
pub const MARIADB_DATABASE_PREFIX: &str = "db_";
pub const MARIADB_USER_PREFIX: &str = "user_";

pub const NETWORK_DEFAULT: &str = "novium_daemon_bridge_default";
pub const NETWORK_INTERNET: &str = "novium_daemon_bridge_internet";
pub const NETWORK_IPV6: &str = "novium_daemon_macvlan_ipv6";
pub const NETWORK_PREFIX: &str = "novium-daemon-network_";
pub const IPV4_PREFIX: [u8; 2] = [10, 235];
pub const IPV6_PREFIX: [u16; 3] = [0xfd33, 0x214f, 0x9578];

pub const HOSTNAME: &[&str] = &["daemon_host"];

pub const DAEMON_TERMINAL_BUFFER_LIMIT: usize = 1024 * 1024; // 1 MiB

pub const STORAGE_SCAN_INTERVAL: ::core::time::Duration = ::core::time::Duration::from_mins(30);
pub const SSH_BANNER_PREFIX: &str = "SSH-2.0-Daemon-";

pub const DAEMON_BACKUP_NAME_XATTR: &str = "user.novium_daemon.backup_name";

pub const BRIDGE_DEFAULT_IPV6_PREFIX: &str = "fdcb:c049:22d3:a::/64";
pub const BRIDGE_INTERNET_IPV6_PREFIX: &str = "fdcb:c049:22d3:b::/64";

pub const DAEMON_ROOT_KEY_COMMON_NAME: &str = "Novium CA";
pub const DAEMON_BACKEND_KEY_COMMON_NAME: &str = "Novium Backend";

#[cfg(target_os = "macos")]
pub const DAEMON_DEFAULT_RUNTIME_DIR: &str = "/private/tmp/novium_daemon";

#[cfg(not(target_os = "macos"))]
pub const DAEMON_DEFAULT_RUNTIME_DIR: &str = "/run/novium_daemon";
