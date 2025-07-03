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
