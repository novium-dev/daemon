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
    use crate::service::database::DatabaseUserPermissions;
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
    out += "import { AxiosRequestConfig, AxiosInstance, ResponseType as AxiosResponseType, AxiosError, AxiosResponse } from 'axios';\n";
    out += "import axios from 'axios'\n";
    out += "import * as https from 'node:https';\n";
    out += "import { Readable } from 'node:stream';\n";
    out += "import * as forge from 'node-forge';\n";
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
type DaemonSDKRequest = {{ url: string, method: string, headers: Record<string, string>, data?: any, responseType: AxiosResponseType }};

export class DaemonSDK {{
  private readonly baseUrl: string;
  protected readonly identifier: string;
  private readonly client: AxiosInstance;

  constructor(baseUrl: string, identifier: string, daemonRootKey: string, backendKey: string, headers: Record<string, string> = {{}}) {{
    if (!baseUrl.endsWith("/")) {{
      baseUrl += "/";
    }}
    this.baseUrl = baseUrl;
    this.identifier = identifier;
    this.client = axios.create({{
      headers: headers,
      httpsAgent: new https.Agent({{
        key: backendKey,
        cert: backendKey,
        ca: daemonRootKey,
        servername: `node-${{identifier}}`,
      }}),
    }});
  }}

{}

}}

export class DaemonSDKWithUserKey extends DaemonSDK {{
  private readonly userKey: string | Uint8Array;

  constructor(baseUrl: string, identifier: string, daemonRootKey: string, backendKey: string, headers: Record<string, string> = {{}}) {{
    super(baseUrl, identifier, daemonRootKey, backendKey, headers);
    this.userKey = "";

    let userKey: forge.util.ByteStringBuffer | null = null;
    const daemonRootKeyDecoded = forge.pem.decode(daemonRootKey);
    for (const entry of daemonRootKeyDecoded) {{
      if (entry.type === "PRIVATE USER KEY") {{
        userKey = forge.util.createBuffer(entry.body);
        break;
      }}
    }}
    if (userKey === null) {{
      throw new Error("User Key is not present");
    }}
    const keyAsn1 = forge.asn1.fromDer(userKey as forge.util.ByteStringBuffer);
    const userPrivateKey = forge.pki.ed25519.privateKeyFromAsn1(keyAsn1);

    this.userKey = userPrivateKey.privateKeyBytes;
  }}

{}

}}
    "#,
        visitor.out, user_visitor.out
    );
    out += "\n";

    std::fs::write("sdk.ts", out).unwrap();
}
