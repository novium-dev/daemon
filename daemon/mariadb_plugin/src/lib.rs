mod constants;
mod r#macro;
mod plugin;
mod types;

use crate::types::{
    AuthHandler, AuthStatus, AuthenticateFunc, DeinitFunc, InitFunc, License, Plugin, PluginType,
    PluginVIO, ServerAuthInfo,
};
use constcat::concat_slices;
use std::ffi::{c_int, c_void};

include!("../../../config.rs");

extern "C" fn init(_: *mut c_void) -> c_int {
    err(unsafe { plugin::init() })
}

extern "C" fn deinit(_: *mut c_void) -> c_int {
    err(unsafe { plugin::deinit() })
}

fn err(result: Result<(), i32>) -> c_int {
    match result {
        Ok(_) => 0,
        Err(err) => err,
    }
}

extern "C" fn authenticate_user(vio: *mut PluginVIO, info: *mut ServerAuthInfo) -> AuthStatus {
    unsafe { plugin::authenticate(&mut *vio, &mut *info) }
}

pub const AUTHENTICATION_HANDLER: AuthHandler = AuthHandler {
    interface_version: constants::MARIA_AUTHENTICATION_INTERFACE_VERSION,
    authenticate_user: authenticate_user as *const AuthenticateFunc,
    client_auth_plugin: std::ptr::null(),
    hash_password: std::ptr::null(),
    preprocess_hash: std::ptr::null(),
};

pub const PLUGIN_BYTES: &[u8] = MARIADB_PLUGIN_NAME.as_bytes();
pub const PLUGIN_NAME: &[u8] = concat_slices!([u8]: PLUGIN_BYTES, &[0]);

mysql_declare_plugin! {
    Plugin {
        plugin_type: PluginType::Authentication,
        info: &AUTHENTICATION_HANDLER as *const _ as *const c_void,
        name: PLUGIN_NAME.as_ptr(),
        author: b"MarkusTieger\0" as *const u8,
        descr: b"A authentication plugin\0" as *const u8,
        license: License::Gpl,
        init: init as *const InitFunc,
        deinit: deinit as *const DeinitFunc,
        version: 0x0001,
        status_vars: std::ptr::null(),
        system_vars: std::ptr::null(),
        version_info: b"1.0" as *const u8,
        maturity: constants::MARIA_PLUGIN_MATURITY_STABLE,
    }
}
