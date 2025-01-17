use crate::constants::MARIA_PLUGIN_MATURITY_STABLE;
use std::ffi::{c_char, c_int, c_uchar, c_uint, c_ulong, c_void};
use std::mem::MaybeUninit;

#[non_exhaustive]
#[repr(i32)]
pub enum PluginType {
    /// User-Defined Function.
    Udf = 0,

    /// Storage engine.
    Storage = 1,

    Authentication = 7,
}

/// The license under which the plugin is being published.
#[repr(i32)]
pub enum License {
    /// A proprietary license.
    Proprietary = 0,

    /// The GNU General Public License.
    Gpl = 1,

    /// The BSD license.
    Bsd = 2,
}

#[repr(C)]
pub struct Plugin {
    /// The type of the plugin.
    pub plugin_type: PluginType,

    /// Pointer to one of the supported info structures.
    ///
    /// These are:
    ///  * `StorageEngineInfo`
    pub info: *const c_void,

    /// The name of the plugin.
    pub name: *const u8,

    /// The author (Person or organization).
    pub author: *const u8,

    /// The description.
    pub descr: *const u8,

    /// The license under which the plugin is published.
    pub license: License,

    /// Function to invoke when plugin is loaded.
    pub init: *const InitFunc,

    /// Function to invoke when plugin is unloaded.
    pub deinit: *const DeinitFunc,

    /// Version number of the plugin.
    pub version: c_uint,

    /// TODO: SHOW STATUS Server status variable
    pub status_vars: *const c_void,

    /// TODO: Definition of system vars structure for access their information in the plugin
    pub system_vars: *const c_void,

    /// Flags for the plugin.
    pub version_info: *const u8,

    pub maturity: c_uint,
}

unsafe impl Sync for Plugin {}

impl Plugin {
    /// Create a Plugin information structure with all fields being zero.
    pub const fn zero() -> Self {
        Self {
            plugin_type: PluginType::Udf,
            info: std::ptr::null_mut(),
            name: std::ptr::null(),
            author: std::ptr::null(),
            descr: std::ptr::null(),
            license: License::Proprietary,
            init: std::ptr::null(),
            deinit: std::ptr::null(),
            version: 0,
            status_vars: std::ptr::null(),
            system_vars: std::ptr::null(),
            version_info: std::ptr::null(),
            maturity: MARIA_PLUGIN_MATURITY_STABLE,
        }
    }
}

/// Function to invoke when plugin is loaded.
pub type InitFunc = extern "C" fn(*mut c_void) -> i32;

/// Function to invoke when plugin is unloaded.
pub type DeinitFunc = extern "C" fn(*mut c_void) -> i32;

#[allow(non_camel_case_types)]
pub type size_t = usize;
pub type AuthenticateFunc =
    extern "C" fn(vio: *mut PluginVIOInfo, info: *mut ServerAuthInfo) -> AuthStatus;
pub type HashPasswordFunc = extern "C" fn(
    password: *const char,
    password_length: *const size_t,
    hash: *const char,
    hash_length: *const size_t,
) -> c_int;
pub type PreProcessHashFunc = extern "C" fn(
    hash: *const char,
    hash_length: size_t,
    out: *const c_uchar,
    out_length: *const size_t,
) -> c_int;

#[repr(C)]
pub struct AuthHandler {
    pub interface_version: i32,
    pub client_auth_plugin: *const c_char,
    pub authenticate_user: *const AuthenticateFunc,
    pub hash_password: *const HashPasswordFunc,
    pub preprocess_hash: *const PreProcessHashFunc,
}

#[repr(i32)]
#[allow(non_camel_case_types)]
pub enum AuthStatus {
    Error = 0,
    Ok = -1,
}

#[repr(C)]
#[allow(non_camel_case_types)]
pub enum PluginVIOInfoProtocol {
    MYSQL_VIO_INVALID,
    MYSQL_VIO_TCP,
    MYSQL_VIO_SOCKET,
    MYSQL_VIO_PIPE,
    MYSQL_VIO_MEMORY,
}

#[repr(C)]
pub struct PluginVIOInfo {
    pub protocol: PluginVIOInfoProtocol,
    pub socket: c_int,
}

pub type ReadPacketFunc = extern "C" fn(*mut PluginVIO, *mut *mut c_uchar) -> c_int;
pub type WritePacketFunc = extern "C" fn(*mut PluginVIO, *const c_uchar, c_int) -> c_int;
pub type VIOInfoFunc = extern "C" fn(*mut PluginVIO, *mut PluginVIOInfo);

#[repr(C)]
pub struct PluginVIO {
    pub read_packet: Option<ReadPacketFunc>,
    pub write_packet: Option<WritePacketFunc>,
    pub info: Option<VIOInfoFunc>,
}

impl PluginVIO {
    pub fn read_packet(&mut self, buf: &mut MaybeUninit<*mut c_uchar>) -> c_int {
        (self.read_packet.as_ref().unwrap())(self, buf.as_mut_ptr())
    }

    #[allow(unused)]
    pub fn write_packet(&mut self, packet: &c_uchar, packet_len: c_int) -> c_int {
        (self.write_packet.as_ref().unwrap())(self, packet, packet_len)
    }

    pub fn info(&mut self, info: &mut MaybeUninit<PluginVIOInfo>) {
        (self.info.as_ref().unwrap())(self, info.as_mut_ptr())
    }
}

pub const MYSQL_USERNAME_LENGTH: usize = 512;

pub const PASSWORD_USED_NO_MENTION: c_int = 2;

#[repr(C)]
pub struct ServerAuthInfo {
    pub user_name: *const c_char,
    pub user_name_length: c_uint,
    pub auth_string: *const c_char,
    pub auth_string_length: c_ulong,
    pub authenticated_as: [c_char; MYSQL_USERNAME_LENGTH + 1],
    pub external_user: [c_char; MYSQL_USERNAME_LENGTH + 1],
    pub password_used: c_int,
    pub host_or_ip: *const c_char,
    pub host_or_ip_length: c_uint,
}
