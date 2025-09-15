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
use super::*;
use crate::service::service;
use crate::user::*;
use chrono::Utc;
use zelus::define_query_variable;
use zelus::responses::FileResponse;
use serde::{Deserialize, Serialize};
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
    /// Lists all backups of an existing server
    #[route("", method = GET)]
    #[error(storage(not_found mutex_locked failure [io:*]) sql(failure [sql:*]))]
    async fn server_list_backups(&self, server_id: u32) -> Result<Vec<BackupResponseEntry>, _>;

    /// Creates a new backup (selecting the backup storage by order)
    #[route("", method = PUT)]
    #[error(storage(not_found mutex_locked failure [io:*]) backup_storage(no_suitable_found) sql(failure [sql:*]))]
    async fn server_create_backup_order(&self, server_id: u32) -> Result<i64, _>;

    /// Creates a new backup
    #[route("/{backup_storage}", method = PUT)]
    #[error(storage(not_found mutex_locked failure [io:*]) backup_storage(incompatible location_not_found) sql(failure [sql:*]))]
    async fn server_create_backup(&self, server_id: u32, backup_storage: String) -> Result<i64, _>;

    /// Downloads a backup
    #[route("/{backup_storage}/{backup_id}", method = GET, raw)]
    #[error(storage(not_found mutex_locked failure [io:*]) backup_storage(location_not_found) sql(failure [sql:*]))]
    async fn server_download_backup(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
    ) -> Result<FileResponse, _>;

    /// Downloads a backup (called by the user)
    #[route(!"/user/backup/{server_id}/{backup_storage}/{backup_id}", method = GET, query = [ timestamp, verify ], raw, no_auth)]
    #[error(auth(signature_invalid signature_expired) storage(not_found mutex_locked failure [io:*]) backup_storage(location_not_found) sql(failure [sql:*]))]
    async fn server_download_backup_user(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<FileResponse, _>;

    /// Downloads the contents of a server in realtime (without creating a backup)
    #[route("/rt", method = GET, query = [ compression ], raw)]
    #[error(storage(not_found mutex_locked failure[io:*]) compression(unsupported) sql(failure [sql:*]))]
    async fn server_download_realtime(
        &self,
        server_id: u32,
        compression: String,
    ) -> Result<FileResponse, _>;

    /// Downloads the contents of a server in realtime (without creating a backup and called by the user)
    #[route(!"/user/backup/{server_id}/rt", method = GET, query = [ timestamp, verify, compression ], raw, no_auth)]
    #[error(auth(signature_invalid signature_expired) storage(not_found mutex_locked failure[io:*]) compression(unsupported) sql(failure [sql:*]))]
    async fn server_download_realtime_user(
        &self,
        server_id: u32,
        timestamp: i64,
        verify: UserSignature,
        compression: String,
    ) -> Result<FileResponse, _>;

    /// Sets the name of a backup
    #[route("/{backup_storage}/{backup_id}", method = PATCH)]
    #[error(storage(not_found mutex_locked failure [io:*]) backup(not_found [io:NotFound]) backup_storage(location_not_found) sql(failure [sql:*]))]
    async fn server_set_backup_name(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
        name: Option<String>,
    ) -> Result<(), _>;

    /// Deletes a backup
    #[route("/{backup_storage}/{backup_id}", method = DELETE)]
    #[error(storage(not_found mutex_locked failure[io:*]) backup(not_found [io:NotFound]) backup_storage(location_not_found) sql(failure [sql:*]))]
    async fn server_delete_backup(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
    ) -> Result<(), _>;

    /// Restores a backup
    #[route("/{backup_storage}/{backup_id}/restore", method = GET)]
    #[error(storage(not_found mutex_locked failure [io:*] quota_reached [io:QuotaExceeded]) backup(not_found [io:NotFound]) backup_storage(location_not_found) sql(failure [sql:*]))]
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

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({"time": 1_731_160_477_597i64, "storage": "local"}))]
pub struct BackupResponseEntry {
    pub time: i64,
    pub storage: String,
    pub name: Option<String>,
    pub size: Option<u64>,
}
