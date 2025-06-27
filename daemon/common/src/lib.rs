#![forbid(unsafe_code)]

pub mod files;
#[cfg(feature = "keys")]
pub mod key;
#[cfg(feature = "sdk")]
pub mod sdk;

#[cfg(feature = "backend")]
pub mod node;
#[cfg(feature = "service")]
pub mod service;
#[cfg(feature = "ssh")]
pub mod ssh;
#[cfg(feature = "service")]
pub mod user;
pub mod utils;

include!("../../../config.rs");

#[cfg(feature = "ts")]
#[test]
fn export_ts() {
    use crate::files::FileMode;
    use crate::sdk::DaemonSDKWithUserKey;
    use crate::sdk::DefaultDaemonSDK;
    use crate::service::backup::BackupService;
    use crate::service::database::DatabaseService;
    use crate::service::files::FilesService;
    use crate::service::network::NetworkService;
    use crate::service::resource::ResourceService;
    use crate::service::schedule::ScheduleService;
    use crate::service::server::ServerService;
    use crate::user::UserSignature;

    use crate::service::backup::BackupServiceUserKeyExtension;
    use crate::service::database::DatabaseUserPermissions;
    use crate::service::files::FilesServiceUserKeyExtension;
    use crate::service::server::ServerServiceUserKeyExtension;

    let mut type_visitor = daemon_framework::ts::TSTypeVisitor::default();

    daemon_framework::framework_ts_type_export!(DefaultDaemonSDK type_visitor {
        ServerService, BackupService, FilesService, NetworkService,
        ResourceService, DatabaseService, ScheduleService
    });

    DatabaseUserPermissions::visit_types(&mut type_visitor);
    FileMode::visit_types(&mut type_visitor);
    UserSignature::visit_types(&mut type_visitor);

    let mut visitor = daemon_framework::ts::TSClassFunctionVisitor::default();

    daemon_framework::framework_ts_func_export!(DefaultDaemonSDK visitor {
        ServerService, BackupService, FilesService, NetworkService,
        ResourceService, DatabaseService, ScheduleService
    });

    let mut user_visitor = daemon_framework::ts::TSClassFunctionVisitor::default();
    <DaemonSDKWithUserKey as ServerServiceUserKeyExtension>::visit_funcs(&mut user_visitor);
    <DaemonSDKWithUserKey as BackupServiceUserKeyExtension>::visit_funcs(&mut user_visitor);
    <DaemonSDKWithUserKey as FilesServiceUserKeyExtension>::visit_funcs(&mut user_visitor);

    let sdk = include_str!("sdk/template.ts")
        .replace("/** {types} */", &type_visitor.out)
        .replace("/** {sdk_implementation} */", &visitor.out)
        .replace("/** {sdk_user_implementation} */", &user_visitor.out)
        .replace("/** {sdk_config_consts} */", &format!("export const DAEMON_ROOT_KEY_COMMON_NAME: string = {};\nexport const DAEMON_BACKEND_KEY_COMMON_NAME: string = {};\n", serde_json::to_string(&crate::DAEMON_ROOT_KEY_COMMON_NAME).unwrap(), serde_json::to_string(&crate::DAEMON_BACKEND_KEY_COMMON_NAME).unwrap()));

    std::fs::write("sdk.ts", sdk).unwrap();
}
