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
use crate::service::server::ServerFileWrite;
use crate::service::service;
use crate::user::*;
use chrono::Utc;
use daemon_framework::axum_extra::TypedHeader;
use daemon_framework::axum_extra::headers::ContentLength;
use daemon_framework::define_query_variable;
use daemon_framework::responses::FileResponse;
use daemon_framework::types::DataStream;
use serde::{Deserialize, Serialize};
use url::Url;

define_error!(file {
    not_found ("This file doesn't exist" NOT_FOUND),
    not_directory ("The path does not point to a directory" FORBIDDEN),
    is_directory ("The path does point to a directory" FORBIDDEN),
    already_exists ("There is already a file on this path" FORBIDDEN),
    sandbox_violated ("You tried access files you aren't allowed to" FORBIDDEN),
    limit_reached ("Your specified limit has been exceeded" PAYLOAD_TOO_LARGE),
});
define_query_variable!(path "The location of the file or directory");
define_query_variable!(limit "The file size limit");
define_header_variable!(length "The content length of the upload");

#[service(path = "/files/{server_id}", tag = "files")]
pub trait FilesService {
    #[route("/list", method = GET, query = [ path ])]
    #[error(storage(not_found mutex_locked failure [io:*]) file(not_found [io:NotFound] sandbox_violated [io:PermissionDenied] not_directory [io:NotADirectory]) sql(failure [sql:*]))]
    #[doc = "List files inside a directory"]
    async fn server_list_files(&self, server_id: u32, path: String) -> Result<Vec<FileEntry>, _>;

    #[route("/stat", method = GET, query = [ path ])]
    #[error(storage(not_found mutex_locked failure [io:*]) file(not_found [io:NotFound] sandbox_violated [io:PermissionDenied]) sql(failure [sql:*]))]
    #[doc = "Gets the info about a file"]
    async fn server_stat_file(&self, server_id: u32, path: String) -> Result<FileEntry, _>;

    #[route("/rename", method = GET, query = [ from "The old file location", to "The new file location" ])]
    #[error(storage(not_found mutex_locked failure [io:*]) file(not_found [io:NotFound] sandbox_violated [io:PermissionDenied] already_exists [io:AlreadyExists]) sql(failure [sql:*]))]
    #[doc = "Renames/Moves a file"]
    async fn server_rename_file(&self, server_id: u32, from: String, to: String) -> Result<(), _>;

    #[route("/copy", method = GET, query = [ from "The original file location", to "The new copied file location" ])]
    #[error(storage(not_found mutex_locked failure [io:*] quota_reached [io:QuotaExceeded]) file(not_found [io:NotFound] sandbox_violated [io:PermissionDenied] is_directory [io:IsADirectory]) sql(failure [sql:*]))]
    #[doc = "Copies a file"]
    async fn server_copy_file(&self, server_id: u32, from: String, to: String) -> Result<(), _>;

    #[route("/file", method = GET, query = [ path, limit ], raw)]
    #[error(storage(not_found mutex_locked failure [io:*]) file(not_found [io:NotFound] sandbox_violated [io:PermissionDenied] is_directory [io:IsADirectory] limit_reached) sql(failure [sql:*]))]
    #[doc = "Downloads a file"]
    async fn server_download_file(
        &self,
        server_id: u32,
        path: String,
        limit: Option<u64>,
    ) -> Result<FileResponse, _>;

    #[route(!"/user/files/{server_id}/file", method = GET, query = [ path, limit, timestamp, verify ], user, raw)]
    #[error(auth(signature_invalid signature_expired) storage(not_found mutex_locked failure [io:*]) file(not_found [io:NotFound] sandbox_violated [io:PermissionDenied] is_directory [io:IsADirectory] limit_reached) sql(failure [sql:*]))]
    #[doc = "Downloads a file (called by the user)"]
    async fn server_download_file_user(
        &self,
        server_id: u32,
        path: String,
        limit: Option<u64>,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<FileResponse, _>;

    #[route("/file", method = PUT, query = [ path ])]
    #[error(storage(not_found mutex_locked failure [io:*] quota_reached [io:QuotaExceeded]) file(not_found [io:NotFound] sandbox_violated [io:PermissionDenied] is_directory [io:IsADirectory]) sql(failure [sql:*]))]
    #[doc = "Uploads a file"]
    async fn server_upload_file(
        &self,
        server_id: u32,
        path: String,
        length: Option<TypedHeader<ContentLength>>,
        data: DataStream,
    ) -> Result<(), _>;

    #[route(!"/user/files/{server_id}/file", method = PUT, query = [ path, timestamp, verify ], user)]
    #[error(auth(signature_invalid signature_expired) storage(not_found mutex_locked failure [io:*] quota_reached [io:QuotaExceeded]) file(not_found [io:NotFound] sandbox_violated [io:PermissionDenied] is_directory [io:IsADirectory]) sql(failure [sql:*]))]
    #[doc = "Uploads a file (called by the user)"]
    async fn server_upload_file_user(
        &self,
        server_id: u32,
        path: String,
        timestamp: i64,
        verify: UserSignature,
        length: Option<TypedHeader<ContentLength>>,
        data: DataStream,
    ) -> Result<(), _>;

    #[route("/url", method = GET, query = [ path, url "The url from which the file gets downloaded" ])]
    #[error(storage(not_found mutex_locked failure [io:*] quota_reached [io:QuotaExceeded]) file(not_found [io:NotFound] sandbox_violated [io:PermissionDenied] is_directory [io:IsADirectory]) sql(failure [sql:*]) http_request(failed not_success read_failed))]
    #[doc = "Writes a file using a url"]
    async fn server_write_file_by_url(
        &self,
        server_id: u32,
        path: String,
        url: Url,
    ) -> Result<(), _>;

    #[route("/template", method = POST, query = [ path ])]
    #[error(storage(not_found mutex_locked failure [io:*] quota_reached [io:QuotaExceeded] patch_failed) file(not_found [io:NotFound] sandbox_violated [io:PermissionDenied] is_directory [io:IsADirectory]) sql(failure [sql:*]))]
    #[doc = "Writes a file using a template"]
    async fn server_write_file_by_template(
        &self,
        server_id: u32,
        path: String,
        template: ServerFileWrite,
    ) -> Result<(), _>;

    #[route("/file", method = DELETE, query = [ path, recursive "If it is a directory and should be deleted recursively" ])]
    #[error(storage(not_found mutex_locked failure [io:*]) file(not_found [io:NotFound] sandbox_violated [io:PermissionDenied] not_directory [io:NotADirectory] is_directory [io:IsADirectory]) sql(failure [sql:*]))]
    #[doc = "Deletes a file"]
    async fn server_delete_file(
        &self,
        server_id: u32,
        path: String,
        recursive: bool,
    ) -> Result<(), _>;

    #[route("/mkdir", method = GET, query = [ path, recursive "If parents should be created as well" ])]
    #[error(storage(not_found mutex_locked failure [io:*] quota_reached [io:QuotaExceeded]) file(not_found [io:NotFound io:NotADirectory] sandbox_violated [io:PermissionDenied] already_exists [io:AlreadyExists]) sql(failure [sql:*]))]
    #[doc = "Creates a new directory"]
    async fn server_create_directory(
        &self,
        server_id: u32,
        path: String,
        recursive: bool,
    ) -> Result<(), _>;

    #[route("/symlink", method = GET, query = [ path, content "The contents of the symlink" ])]
    #[error(storage(not_found mutex_locked failure [io:*] quota_reached [io:QuotaExceeded]) file(not_found [io:NotFound] sandbox_violated [io:PermissionDenied] already_exists [io:AlreadyExists]) sql(failure [sql:*]))]
    #[doc = "Creates a new symlink"]
    async fn server_symlink(&self, server_id: u32, path: String, content: String) -> Result<(), _>;

    #[route("/archive/unpack", method = GET, query = [ path "The location of the existing archive", dest "The destination directory location" ])]
    #[error(storage(not_found mutex_locked failure [io:*] quota_reached [io:QuotaExceeded]) file(not_found [io:NotFound] sandbox_violated [io:PermissionDenied]) compression(unsupported) sql(failure [sql:*]))]
    #[doc = "Unpacks an existing archive"]
    async fn server_unpack_archive(
        &self,
        server_id: u32,
        path: String,
        dest: String,
    ) -> Result<(), _>;

    #[route("/archive/create", method = GET, query = [
        path "The source directory for the archive contents",
        src "The file location where the archive should be created",
        compression "The compression algorithm to use for the archive"
    ])]
    #[error(storage(not_found mutex_locked failure [io:*] quota_reached [io:QuotaExceeded]) file(not_found [io:NotFound] sandbox_violated [io:PermissionDenied] is_directory [io:IsADirectory]) compression(unsupported) sql(failure [sql:*]))]
    #[doc = "Creates an archive"]
    async fn server_create_archive(
        &self,
        server_id: u32,
        src: String,
        path: String,
        compression: String,
    ) -> Result<(), _>;

    #[route("/watch", method = WS, query = [ path "The initial work directory path" ])]
    #[error(storage(not_found mutex_locked failure [io:*]) sql(failure [sql:*]))]
    #[doc = "A websocket for file updates in realtime"]
    async fn server_watch_files(
        &self,
        server_id: u32,
        path: Option<String>,
    ) -> Result<WebsocketResponse, _>;

    #[route(!"/user/files/{server_id}/watch", method = WS, query = [ path "The initial work directory path", verify, timestamp ])]
    #[error(auth(signature_invalid signature_expired) storage(not_found mutex_locked failure [io:*]) sql(failure [sql:*]))]
    #[doc = "A websocket for file updates in realtime (called by the user)"]
    async fn server_watch_files_user(
        &self,
        server_id: u32,
        path: Option<String>,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, _>;

    #[route("/wipe", method = GET)]
    #[error(storage(not_found mutex_locked failure [io:*]) sql(failure [sql:*]))]
    #[doc = "Deletes all files"]
    async fn wipe_server(&self, server_id: u32);
}

pub trait FilesServiceUserKeyExtension {
    fn server_download_file_user_url_signed(
        &self,
        server_id: u32,
        path: String,
        limit: Option<u64>,
    ) -> Url;

    fn server_upload_file_user_url_signed(&self, server_id: u32, path: String) -> Url;

    fn server_watch_files_user_url_signed(&self, server_id: u32, path: Option<String>) -> Url;
}

impl<T: FilesServiceURL + UserKeyProvider> FilesServiceUserKeyExtension for T {
    fn server_download_file_user_url_signed(
        &self,
        server_id: u32,
        path: String,
        limit: Option<u64>,
    ) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature = UserSignature::new_server_file_download(
            self.key(),
            timestamp,
            self.node_id(),
            server_id,
            &path,
        );
        self.server_download_file_user_url(server_id, path, limit, timestamp, signature)
    }

    fn server_upload_file_user_url_signed(&self, server_id: u32, path: String) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature = UserSignature::new_server_file_upload(
            self.key(),
            timestamp,
            self.node_id(),
            server_id,
            &path,
        );
        self.server_upload_file_user_url(server_id, path, timestamp, signature)
    }

    fn server_watch_files_user_url_signed(&self, server_id: u32, path: Option<String>) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature =
            UserSignature::new_server_watch_files(self.key(), timestamp, self.node_id(), server_id);
        self.server_watch_files_user_url(server_id, path, timestamp, signature)
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({"name": "eula.txt", "uid": 0, "gid": 0, "mode": 0o644, "size": 64, "path": null}))]
pub struct FileEntry {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub mode: u32,
    pub size: u64,
    pub relative_path: String, // for recursive listing like /plugins/essentials
    pub path: Option<String>,  // for symlinks
    pub archive: bool,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct FilesWatchResponse {
    pub token: String,
    pub data: Vec<FilesWatchResponseEntry>,
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "type")]
#[serde(rename_all = "lowercase")]
pub enum FilesWatchCommand {
    Reload,
    Fresh,
    WorkDirectory { path: String },
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum FilesWatchWorkDirectoryChangedReason {
    Changed, // another path was requested via command
    Update,  // the current directory got renamed (still the same directory, just the path differs)
    Delete, // current directory got deleted, forced to change into another directory (if possible the parent directory, otherwise root)
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum FilesWatchResponseEntry {
    WorkDirectoryChanged {
        path: String,
        reason: FilesWatchWorkDirectoryChangedReason,
    },
    Reset {
        data: Vec<FileEntry>,
    },
    Add {
        index: usize,
        entry: FileEntry,
    },
    Remove {
        index: usize,
    },
    Modify {
        index: usize,
        entry: FileEntry,
    },
    Move {
        index: usize,
        offset: i64,
    },
}
