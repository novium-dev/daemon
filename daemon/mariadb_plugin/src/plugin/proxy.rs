use crate::plugin::{authenticate_as, identify};
use crate::types::{AuthStatus, PASSWORD_USED_NO_MENTION, PluginVIO, ServerAuthInfo};

pub(crate) fn authenticate_proxy_mode(
    vio: &mut PluginVIO,
    info: &mut ServerAuthInfo,
) -> AuthStatus {
    info.password_used = PASSWORD_USED_NO_MENTION;

    match identify(vio) {
        Ok(container) => {
            #[cfg(debug_assertions)]
            {
                println!("Identified Container as {}", container);
            }
            authenticate_as(info, format!("server_{}", container), true)
        }
        #[allow(unused_variables)]
        Err(err) => {
            #[cfg(debug_assertions)]
            {
                println!("Unable to identify container: {:?}", err);
            }
            AuthStatus::Error
        }
    }
}
