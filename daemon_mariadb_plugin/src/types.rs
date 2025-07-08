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
use crate::constants::MARIA_PLUGIN_MATURITY_STABLE;
use core::ffi::{c_char, c_int, c_uchar, c_uint, c_ulong, c_void};
use core::mem::MaybeUninit;
use libc::size_t;

#[non_exhaustive]
#[repr(i32)]
#[expect(
    dead_code,
    reason = "this is a wrapper implementation, implement every variant there is"
)]
pub enum PluginType {
    /// User-Defined Function.
    Udf = 0,

    /// Storage engine.
    Storage = 1,

    Authentication = 7,
}

/// The license under which the plugin is being published.
#[repr(i32)]
#[expect(
    dead_code,
    reason = "this is a wrapper implementation, implement every variant there is"
)]
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

    pub status_vars: *const c_void,

    pub system_vars: *const c_void,

    /// Flags for the plugin.
    pub version_info: *const u8,

    pub maturity: c_uint,
}

// SAFETY: It can be shared between threads
unsafe impl Sync for Plugin {}

impl Plugin {
    /// Create a Plugin information structure with all fields being zero.
    pub const fn zero() -> Self {
        Self {
            plugin_type: PluginType::Udf,
            info: core::ptr::null_mut(),
            name: core::ptr::null(),
            author: core::ptr::null(),
            descr: core::ptr::null(),
            license: License::Proprietary,
            init: core::ptr::null(),
            deinit: core::ptr::null(),
            version: 0,
            status_vars: core::ptr::null(),
            system_vars: core::ptr::null(),
            version_info: core::ptr::null(),
            maturity: MARIA_PLUGIN_MATURITY_STABLE,
        }
    }
}

/// Function to invoke when plugin is loaded.
pub type InitFunc = extern "C" fn(*mut c_void) -> i32;

/// Function to invoke when plugin is unloaded.
pub type DeinitFunc = extern "C" fn(*mut c_void) -> i32;

pub type AuthenticateFunc =
    extern "C" fn(vio: *mut PluginVIO, info: *mut ServerAuthInfo) -> AuthStatus;
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
    pub interface_version: u32,
    pub client_auth_plugin: *const c_char,
    pub authenticate_user: *const AuthenticateFunc,
    pub hash_password: *const HashPasswordFunc,
    pub preprocess_hash: *const PreProcessHashFunc,
}

#[repr(i32)]
pub enum AuthStatus {
    Error = 0,
    Ok = -1,
}

#[repr(C)]
#[expect(non_camel_case_types, reason = "from mariadb header files")]
#[expect(
    dead_code,
    reason = "this is a wrapper implementation, implement every variant there is"
)]
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
    pub read_packet: ReadPacketFunc,
    pub write_packet: WritePacketFunc,
    pub info: VIOInfoFunc,
}

#[expect(
    dead_code,
    reason = "this is a wrapper implementation, implement every function there is"
)]
impl PluginVIO {
    pub fn read_packet(&mut self, buf: &mut MaybeUninit<*mut c_uchar>) -> c_int {
        (self.read_packet)(self, buf.as_mut_ptr())
    }

    pub fn write_packet(&mut self, packet: &c_uchar, packet_len: c_int) -> c_int {
        (self.write_packet)(self, packet, packet_len)
    }

    pub fn info(&mut self, info: &mut MaybeUninit<PluginVIOInfo>) {
        (self.info)(self, info.as_mut_ptr());
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
    pub authenticated_as: [u8; MYSQL_USERNAME_LENGTH + 1],
    pub external_user: [u8; MYSQL_USERNAME_LENGTH + 1],
    pub password_used: c_int,
    pub host_or_ip: *const c_char,
    pub host_or_ip_length: c_uint,
}
