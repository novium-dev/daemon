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
            AuthStatus::Error
        }
    }
}
