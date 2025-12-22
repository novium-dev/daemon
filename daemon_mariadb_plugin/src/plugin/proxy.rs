// SPDX-License-Identifier: AGPL-3.0-only
use crate::plugin::{authenticate_as, identify};
use crate::types::{AuthStatus, PASSWORD_USED_NO_MENTION, PluginVIO, ServerAuthInfo};

pub fn authenticate_proxy_mode(vio: &mut PluginVIO, info: &mut ServerAuthInfo) -> AuthStatus {
    info.password_used = PASSWORD_USED_NO_MENTION;

    match identify(vio) {
        Ok(container) => {
            #[cfg(debug_assertions)]
            {
                println!("Identified Container as {container}");
            };
            authenticate_as(info, format!("server_{container}"), true)
        }
        Err(err) => {
            #[cfg(debug_assertions)]
            {
                println!("Unable to identify container: {err:?}");
            };
            #[cfg(not(debug_assertions))]
            {
                let _ = err;
            };
            AuthStatus::Error
        }
    }
}
