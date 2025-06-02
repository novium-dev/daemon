#![warn(clippy::string_to_string)]
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
    use crate::service::files::FilesServiceUserKeyExtension;
    use crate::service::server::ServerServiceUserKeyExtension;

    let mut visitor = daemon_framework::ts::TSTypeVisitor::default();

    daemon_framework::framework_ts_type_export!(DefaultDaemonSDK visitor {
        ServerService, BackupService, FilesService, NetworkService,
        ResourceService, DatabaseService, ScheduleService
    });

    DatabaseUserPermissions::visit_types(&mut visitor);
    FileMode::visit_types(&mut visitor);
    UserSignature::visit_types(&mut visitor);

    let mut out = String::new();
    out += "import * as ed from '@noble/ed25519';\n";
    out += "\n";
    out += &visitor.out;
    out += "\n";
    let mut visitor = daemon_framework::ts::TSClassFunctionVisitor::default();

    daemon_framework::framework_ts_func_export!(DefaultDaemonSDK visitor {
        ServerService, BackupService, FilesService, NetworkService,
        ResourceService, DatabaseService, ScheduleService
    });

    let mut user_visitor = daemon_framework::ts::TSClassFunctionVisitor::default();
    <DaemonSDKWithUserKey as ServerServiceUserKeyExtension>::visit_funcs(&mut user_visitor);
    <DaemonSDKWithUserKey as BackupServiceUserKeyExtension>::visit_funcs(&mut user_visitor);
    <DaemonSDKWithUserKey as FilesServiceUserKeyExtension>::visit_funcs(&mut user_visitor);

    out += "\n";
    out += &format!(
        r#"
type DaemonSDKRequestInit = {{ method: string, headers: Record<string, string>, body?: BodyInit }};

export class DaemonSDK {{
  private readonly baseUrl: string;
  protected readonly identifier: string;
  private readonly headers: Record<string, string>;

  constructor(baseUrl: string, identifier: string, token: string | null, headers: Record<string, string> = {{}}) {{
    if (!baseUrl.endsWith("/")) {{
      baseUrl += "/";
    }}
    this.baseUrl = baseUrl;
    this.identifier = identifier;
    if (token != null) {{
      headers['Authorization'] = `Bearer ${{token}}`;
    }}
    this.headers = headers;
  }}

{}

}}

export class DaemonSDKWithUserKey extends DaemonSDK {{
  private readonly userKey: string | Uint8Array;

  constructor(baseUrl: string, identifier: string, token: string | null, userKey: string | Uint8Array, headers: Record<string, string> = {{}}) {{
    super(baseUrl, identifier, token, headers);
    this.userKey = userKey;
  }}

{}

}}
    "#,
        visitor.out, user_visitor.out
    );
    out += "\n";

    std::fs::write("sdk.ts", out).unwrap();
}
