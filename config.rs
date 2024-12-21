
pub const LABEL_NAME: &str = "cloud.novium.daemon.server";
pub const IMAGE_NAME: &str = "novium-daemon-image";
pub const CONTAINER_PREFIX: &str = "novium-daemon_";
pub const MARIADB_PLUGIN_NAME: &str = "novium_daemon_auth";
pub const MARIADB_DATABASE_PREFIX: &str = "db_";
pub const MARIADB_USER_PREFIX: &str = "user_";
pub const MARIADB_AUTH_HASH_SIZE: usize = 256;
pub const MARIADB_AUTH_HASH_HEX_SIZE: usize = MARIADB_AUTH_HASH_SIZE / 4;
pub const CACHE_KEY_PREFIX: &str = "novium:";

pub const NETWORK_DEFAULT: &str = "novium_daemon_bridge_default";
pub const NETWORK_INTERNET: &str = "novium_daemon_bridge_internet";
pub const NETWORK_IPV6: &str = "novium_daemon_macvlan_ipv6";
pub const NETWORK_PREFIX: &str = "novium-daemon-network_";
pub const IPV4_PREFIX: [u8; 2] = [10, 235];
pub const IPV6_PREFIX: [u16; 3] = [0xfd33, 0x214f, 0x9578];
