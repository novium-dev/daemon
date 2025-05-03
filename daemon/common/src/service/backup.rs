use super::*;
use crate::payload::backup::BackupResponseEntry;
use crate::service::service;
use crate::user::*;
use chrono::Utc;
use daemon_framework::{FileResponse, define_query_variable};
use url::Url;

define_path_variable!(
    backup_storage "The storage where the backup will be/is stored"
);
define_path_variable!(
    backup_id "The timestamp of the backup. It is used as id"
);
define_query_variable!(
    compression "The compression algorithm to use for the stream"
);

define_error!(backup {
    not_found ( "The backup does not exist" NOT_FOUND )
});

define_error!(backup_storage {
    incompatible (
        "The backup storage cannot be used with the current server storage" FORBIDDEN
    ),
    no_suitable_found (
        "No compatible backup storage for the current server storage could be found" FORBIDDEN
    ),
    location_not_found (
        "The backup storage location does not exist" BAD_REQUEST
    )
});

#[service(path = "/backup/{server_id}", tag = "backup")]
pub trait BackupService {
    #[route("", method = GET)]
    #[error(storage(not_found mutex_locked failure [io:*]) sql(failure [sql:*]))]
    #[doc = "Lists all backups of an existing server"]
    async fn server_list_backups(&self, server_id: u32) -> Result<Vec<BackupResponseEntry>, _>;

    #[route("", method = PUT)]
    #[error(storage(not_found mutex_locked failure [io:*]) backup_storage(no_suitable_found) sql(failure [sql:*]))]
    #[doc = "Creates a new backup (selecting the backup storage by order)"]
    async fn server_create_backup_order(&self, server_id: u32) -> Result<i64, _>;

    #[route("/{backup_storage}", method = PUT)]
    #[error(storage(not_found mutex_locked failure [io:*]) backup_storage(incompatible location_not_found) sql(failure [sql:*]))]
    #[doc = "Creates a new backup"]
    async fn server_create_backup(&self, server_id: u32, backup_storage: String) -> Result<i64, _>;

    #[route("/{backup_storage}/{backup_id}", method = GET, raw)]
    #[error(storage(not_found mutex_locked failure [io:*]) backup_storage(location_not_found) sql(failure [sql:*]))]
    #[doc = "Downloads a backup"]
    async fn server_download_backup(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
    ) -> Result<FileResponse, _>;

    #[route(!"/user/backup/{server_id}/{backup_storage}/{backup_id}", method = GET, query = [ timestamp, verify ], raw, user)]
    #[error(auth(signature_invalid signature_expired) storage(not_found mutex_locked failure [io:*]) backup_storage(location_not_found) sql(failure [sql:*]))]
    #[doc = "Downloads a backup (called by the user)"]
    async fn server_download_backup_user(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<FileResponse, _>;

    #[route("/rt", method = GET, query = [ compression ], raw)]
    #[error(storage(not_found mutex_locked failure[io:*]) compression(unsupported) sql(failure [sql:*]))]
    #[doc = "Downloads the contents of a server in realtime (without creating a backup)"]
    async fn server_download_realtime(
        &self,
        server_id: u32,
        compression: String,
    ) -> Result<FileResponse, _>;

    #[route(!"/user/backup/{server_id}/rt", method = GET, query = [ timestamp, verify, compression ], raw, user)]
    #[error(auth(signature_invalid signature_expired) storage(not_found mutex_locked failure[io:*]) compression(unsupported) sql(failure [sql:*]))]
    #[doc = "Downloads the contents of a server in realtime (without creating a backup and called by the user)"]
    async fn server_download_realtime_user(
        &self,
        server_id: u32,
        timestamp: i64,
        verify: UserSignature,
        compression: String,
    ) -> Result<FileResponse, _>;

    #[route("/{backup_storage}/{backup_id}", method = PATCH)]
    #[error(storage(not_found mutex_locked failure [io:*]) backup(not_found [io:NotFound]) backup_storage(location_not_found) sql(failure [sql:*]))]
    #[doc = "Sets the name of a backup"]
    async fn server_set_backup_name(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
        name: Option<String>,
    ) -> Result<(), _>;

    #[route("/{backup_storage}/{backup_id}", method = DELETE)]
    #[error(storage(not_found mutex_locked failure[io:*]) backup(not_found [io:NotFound]) backup_storage(location_not_found) sql(failure [sql:*]))]
    #[doc = "Deletes a backup"]
    async fn server_delete_backup(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
    ) -> Result<(), _>;

    #[route("/{backup_storage}/{backup_id}/restore", method = GET)]
    #[error(storage(not_found mutex_locked failure [io:*] quota_reached [io:QuotaExceeded]) backup(not_found [io:NotFound]) backup_storage(location_not_found) sql(failure [sql:*]))]
    #[doc = "Restores a backup"]
    async fn server_restore_backup(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
    ) -> Result<(), _>;
}

pub trait BackupServiceUserKeyExtension {
    fn server_download_backup_user_url_signed(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
    ) -> Url;

    fn server_download_realtime_user_url_signed(&self, server_id: u32, compression: String) -> Url;
}

impl<T: BackupServiceURL + UserKeyProvider> BackupServiceUserKeyExtension for T {
    fn server_download_backup_user_url_signed(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
    ) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature = UserSignature::new_server_backup_download(
            self.key(),
            timestamp,
            self.node_id(),
            server_id,
            &backup_storage,
            backup_id,
        );
        self.server_download_backup_user_url(
            server_id,
            backup_storage,
            backup_id,
            timestamp,
            signature,
        )
    }

    fn server_download_realtime_user_url_signed(&self, server_id: u32, compression: String) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature = UserSignature::new_server_realtime_download(
            self.key(),
            timestamp,
            self.node_id(),
            server_id,
            &compression,
        );
        self.server_download_realtime_user_url(server_id, timestamp, signature, compression)
    }
}
