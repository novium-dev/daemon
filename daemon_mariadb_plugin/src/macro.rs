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
#[macro_export]
macro_rules! mysql_declare_plugin {
    ($( $plugin:expr ),+ $(,)?) => {
        #[unsafe(no_mangle)]
        pub static _maria_plugin_interface_version_: u32 = $crate::constants::MARIA_PLUGIN_INTERFACE_VERSION;

        #[unsafe(no_mangle)]
        pub static _maria_sizeof_struct_st_plugin_: u32 = core::mem::size_of::<$crate::types::Plugin>() as u32;

        #[unsafe(no_mangle)]
        pub static _maria_plugin_declarations_: [
            $crate::types::Plugin;
            <[$crate::types::Plugin]>::len(&[$( $plugin, )*]) + 1
        ] = [
            $( $plugin, )*
            $crate::types::Plugin::zero()
        ];
    }
}
