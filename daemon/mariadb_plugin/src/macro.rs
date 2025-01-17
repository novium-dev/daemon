#[macro_export]
macro_rules! mysql_declare_plugin {
    ($( $plugin:expr ),+ $(,)?) => {
        #[no_mangle]
        pub static _maria_plugin_interface_version_: i32 = $crate::constants::MARIA_PLUGIN_INTERFACE_VERSION;

        #[no_mangle]
        pub static _maria_sizeof_struct_st_plugin_: i32 = std::mem::size_of::<$crate::types::Plugin>() as i32;

        #[no_mangle]
        pub static _maria_plugin_declarations_: [
            $crate::types::Plugin;
            <[$crate::types::Plugin]>::len(&[$( $plugin, )*]) + 1
        ] = [
            $( $plugin, )*
            $crate::types::Plugin::zero()
        ];
    }
}
