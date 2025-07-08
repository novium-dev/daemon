use crate::types::{
    AuthHandler, AuthStatus, AuthenticateFunc, DeinitFunc, InitFunc, License, Plugin, PluginType,
    PluginVIO, ServerAuthInfo,
};
use crate::{MARIADB_PLUGIN_NAME, constants, mysql_declare_plugin, plugin};
use constcat::concat_slices;
use core::ffi::{c_int, c_void};

extern "C" fn init(_: *mut c_void) -> c_int {
    err(plugin::init())
}

const extern "C" fn deinit(_: *mut c_void) -> c_int {
    err(plugin::deinit())
}

const fn err(result: Result<(), i32>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(err) => err,
    }
}

extern "C" fn authenticate_user(vio: *mut PluginVIO, info: *mut ServerAuthInfo) -> AuthStatus {
    plugin::authenticate(
        // SAFETY: Passed by mariadb, so should be safe
        unsafe { &mut *vio },
        // SAFETY: Passed by mariadb, so should be safe
        unsafe { &mut *info },
    )
}

pub const AUTHENTICATION_HANDLER: AuthHandler = AuthHandler {
    interface_version: constants::MARIA_AUTHENTICATION_INTERFACE_VERSION,
    authenticate_user: core::ptr::from_ref(&(authenticate_user as AuthenticateFunc)),
    client_auth_plugin: core::ptr::null(),
    hash_password: core::ptr::null(),
    preprocess_hash: core::ptr::null(),
};

pub const PLUGIN_BYTES: &[u8] = MARIADB_PLUGIN_NAME.as_bytes();
pub const PLUGIN_NAME: &[u8] = concat_slices!([u8]: PLUGIN_BYTES, &[0u8]);

mysql_declare_plugin! {
    Plugin {
        plugin_type: PluginType::Authentication,
        info: core::ptr::from_ref(&AUTHENTICATION_HANDLER).cast(),
        name: PLUGIN_NAME.as_ptr(),
        author: core::ptr::from_ref(b"Markus Probst\0").cast(),
        descr: core::ptr::from_ref(b"A authentication plugin\0").cast(),
        license: License::Gpl,
        init: core::ptr::from_ref(&(init as InitFunc)),
        deinit: core::ptr::from_ref(&(deinit as DeinitFunc)),
        version: 0x0001,
        status_vars: core::ptr::null(),
        system_vars: core::ptr::null(),
        version_info: core::ptr::from_ref(b"1.0\0").cast(),
        maturity: constants::MARIA_PLUGIN_MATURITY_STABLE,
    }
}
