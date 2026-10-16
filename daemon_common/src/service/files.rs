use std::collections::HashSet;

// SPDX-License-Identifier: AGPL-3.0-only
use super::*;
use crate::service::server::ServerFileWrite;
use crate::service::service;
use crate::user::*;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use url::Url;
use zelus::define_query_variable;
use zelus::responses::FileResponse;
use zelus::types::DataStream;

pub mod task;

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
    /// List files inside a directory
    #[route("/list", method = GET, query = [ path ])]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*]}, file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied], not_directory <- [io:NotADirectory]}, sql::failure <- [sql:*])]
    async fn server_list_files(&self, server_id: u32, path: String) -> Result<Vec<FileEntry>, _>;

    /// Gets the info about a file
    #[route("/stat", method = GET, query = [ path ])]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*]}, file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied]}, sql::failure <- [sql:*])]
    async fn server_stat_file(&self, server_id: u32, path: String) -> Result<FileEntry, _>;

    /// Renames/Moves a file
    #[route("/rename", method = GET, query = [ from "The old file location", to "The new file location" ])]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*]}, file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied], already_exists <- [io:AlreadyExists]}, sql::failure <- [sql:*])]
    async fn server_rename_file(&self, server_id: u32, from: String, to: String) -> Result<(), _>;

    /// Copies a file
    #[route("/copy", method = GET, query = [ from "The original file location", to "The new copied file location" ])]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*], quota_reached <- [io:QuotaExceeded]},
        file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied], is_directory <- [io:IsADirectory]},
        sql::failure <- [sql:*])]
    async fn server_copy_file(&self, server_id: u32, from: String, to: String) -> Result<(), _>;

    /// Downloads a file
    #[route("/file", method = GET, query = [ path, limit ], raw)]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*]},
        file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied], is_directory <- [io:IsADirectory], limit_reached},
        sql::failure <- [sql:*])]
    async fn server_download_file(
        &self,
        server_id: u32,
        path: String,
        limit: Option<u64>,
    ) -> Result<FileResponse, _>;

    /// Downloads a file (called by the user)
    #[route(!"/user/files/{server_id}/file", method = GET, query = [ path, limit, timestamp, verify ], no_auth, raw)]
    #[error(auth::{signature_invalid, signature_expired},
        storage::{not_found, mutex_locked, failure <- [io:*]},
        file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied], is_directory <- [io:IsADirectory], limit_reached},
        sql::failure <- [sql:*])]
    async fn server_download_file_user(
        &self,
        server_id: u32,
        path: String,
        limit: Option<u64>,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<FileResponse, _>;

    /// Uploads a file
    #[route("/file", method = PUT, query = [ path ])]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*], quota_reached <- [io:QuotaExceeded]},
        file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied], is_directory <- [io:IsADirectory]},
        sql::failure <- [sql:*])]
    async fn server_upload_file(
        &self,
        server_id: u32,
        path: String,
        data: DataStream,
    ) -> Result<(), _>;

    /// Uploads a file (called by the user)
    #[route(!"/user/files/{server_id}/file", method = PUT, query = [ path, timestamp, verify ], no_auth)]
    #[error(auth::{signature_invalid, signature_expired},
        storage::{not_found, mutex_locked, failure <- [io:*], quota_reached <- [io:QuotaExceeded]},
        file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied], is_directory <- [io:IsADirectory]},
        sql::failure <- [sql:*])]
    async fn server_upload_file_user(
        &self,
        server_id: u32,
        path: String,
        timestamp: i64,
        verify: UserSignature,
        data: DataStream,
    ) -> Result<(), _>;

    /// Writes a file using a url
    #[route("/url", method = GET, query = [ path, url "The url from which the file gets downloaded" ])]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*], quota_reached <- [io:QuotaExceeded]},
        file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied], is_directory <- [io:IsADirectory]},
        sql::failure <- [sql:*],
        http_request::{failed, not_success, read_failed})]
    async fn server_write_file_by_url(
        &self,
        server_id: u32,
        path: String,
        #[no_schema] url: Url,
    ) -> Result<(), _>;

    /// Writes a file using a template
    #[route("/template", method = POST, query = [ path ])]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*], quota_reached <- [io:QuotaExceeded], patch_failed},
        file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied], is_directory <- [io:IsADirectory]},
        sql::failure <- [sql:*])]
    async fn server_write_file_by_template(
        &self,
        server_id: u32,
        path: String,
        template: ServerFileWrite,
    ) -> Result<(), _>;

    /// Deletes a file
    #[route("/file", method = DELETE, query = [ path, recursive "If it is a directory and should be deleted recursively" ])]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*]},
        file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied], not_directory <- [io:NotADirectory], is_directory <- [io:IsADirectory]},
        sql::failure <- [sql:*])]
    async fn server_delete_file(
        &self,
        server_id: u32,
        path: String,
        recursive: bool,
    ) -> Result<(), _>;

    /// Creates a new directory
    #[route("/mkdir", method = GET, query = [ path, recursive "If parents should be created as well" ])]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*], quota_reached <- [io:QuotaExceeded]},
        file::{not_found <- [io:NotFound, io:NotADirectory], sandbox_violated <- [io:PermissionDenied], already_exists <- [io:AlreadyExists]},
        sql::failure <- [sql:*])]
    async fn server_create_directory(
        &self,
        server_id: u32,
        path: String,
        recursive: bool,
    ) -> Result<(), _>;

    /// Creates a new symlink
    #[route("/symlink", method = GET, query = [ path, content "The contents of the symlink" ])]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*], quota_reached <- [io:QuotaExceeded]},
        file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied], already_exists <- [io:AlreadyExists]},
        sql::failure <- [sql:*])]
    async fn server_symlink(&self, server_id: u32, path: String, content: String) -> Result<(), _>;

    /// Changes the mode of a file or directory
    #[route("/chmod", method = GET, query = [ path, mode "file mode" ])]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*], quota_reached <- [io:QuotaExceeded]},
        file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied]},
        sql::failure <- [sql:*])]
    async fn server_chmod(&self, server_id: u32, path: String, mode: u32) -> Result<(), _>;

    /// Changes the mode of a file or directory
    #[route("/chown", method = GET, query = [ path, uid "User ID", gid "Group ID" ])]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*], quota_reached <- [io:QuotaExceeded]},
        file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied]},
        sql::failure <- [sql:*])]
    async fn server_chown(
        &self,
        server_id: u32,
        path: String,
        uid: Option<u32>,
        gid: Option<u32>,
    ) -> Result<(), _>;

    /// Unpacks an existing archive
    #[route("/archive/unpack", method = GET, query = [ path "The location of the existing archive", dest "The destination directory location" ])]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*], quota_reached <- [io:QuotaExceeded]},
        file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied]},
        compression::unsupported, sql::failure <- [sql:*])]
    async fn server_unpack_archive(
        &self,
        server_id: u32,
        path: String,
        dest: String,
    ) -> Result<(), _>;

    /// Creates an archive
    #[route("/archive/create", method = POST)]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*], quota_reached <- [io:QuotaExceeded]},
        file::{not_found <- [io:NotFound], sandbox_violated <- [io:PermissionDenied], is_directory <- [io:IsADirectory]},
        compression::unsupported, sql::failure <- [sql:*])]
    async fn server_create_archive(
        &self,
        server_id: u32,
        sources: HashSet<String>,
        exclude: HashSet<String>,
        path: String,
        compression: String,
    ) -> Result<(), _>;

    /// A websocket for file updates in realtime
    #[route("/watch", method = WS, query = [ path "The initial work directory path" ])]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*]}, sql::failure <- [sql:*])]
    async fn server_watch_files(
        &self,
        server_id: u32,
        path: Option<String>,
    ) -> Result<WebsocketResponse, _>;

    /// A websocket for file updates in realtime (called by the user)
    #[route(!"/user/files/{server_id}/watch", method = WS, query = [ path "The initial work directory path", verify, timestamp ])]
    #[error(auth::{signature_invalid, signature_expired}, storage::{not_found, mutex_locked, failure <- [io:*]}, sql::failure <- [sql:*])]
    async fn server_watch_files_user(
        &self,
        server_id: u32,
        path: Option<String>,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<WebsocketResponse, _>;

    /// Writes many files from urls in one call
    ///
    /// Runs the same write as `GET /url` for every file, up to `parallel` at a time, so a
    /// caller with many files needs one request instead of one per file. Every file gets its
    /// own result at the same index, one failed download does not stop the others.
    #[route("/url/batch", method = POST)]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*]}, sql::failure <- [sql:*])]
    async fn server_write_files_by_url(
        &self,
        server_id: u32,
        batch: UrlWriteBatch,
    ) -> Result<Vec<UrlWriteResult>, _>;

    /// Runs many file operations in one call
    ///
    /// Runs the operations in order, each one exactly like its own route (`/list`, `/stat`,
    /// `DELETE /file`, `/rename`, `/copy`, `/mkdir`), so a caller with many operations needs
    /// one request instead of one per operation. Every operation gets its own result at the
    /// same index. With `stop_on_error` the remaining operations are skipped after the first
    /// failure.
    #[route("/batch", method = POST)]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*]}, sql::failure <- [sql:*])]
    async fn server_file_operations(
        &self,
        server_id: u32,
        batch: FileOperationBatch,
    ) -> Result<Vec<FileOperationResult>, _>;

    /// Deletes all files
    #[route("/wipe", method = GET)]
    #[error(storage::{not_found, mutex_locked, failure <- [io:*]}, sql::failure <- [sql:*])]
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

error!(
    BatchOperationError,
    storage::{failure <- [io:*], quota_reached <- [io:QuotaExceeded]},
    file::{
        not_found <- [io:NotFound],
        sandbox_violated <- [io:PermissionDenied],
        already_exists <- [io:AlreadyExists],
        not_directory <- [io:NotADirectory],
        is_directory <- [io:IsADirectory]
    },
    http_request::{failed, not_success, read_failed}
);

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UrlWrite {
    pub path: String,
    #[schema(value_type = String)]
    pub url: Url,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UrlWriteBatch {
    pub files: Vec<UrlWrite>,
    #[serde(default)]
    pub parallel: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FileOperation {
    List {
        path: String,
    },
    Stat {
        path: String,
    },
    Delete {
        path: String,
        #[serde(default)]
        recursive: bool,
    },
    Rename {
        from: String,
        to: String,
    },
    Copy {
        from: String,
        to: String,
    },
    Mkdir {
        path: String,
        #[serde(default)]
        recursive: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct FileOperationBatch {
    pub operations: Vec<FileOperation>,
    #[serde(default)]
    pub stop_on_error: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileOperationFailure {
    StorageFailure,
    QuotaReached,
    NotFound,
    SandboxViolated,
    AlreadyExists,
    NotDirectory,
    IsDirectory,
    HttpRequestFailed,
    HttpRequestNotSuccess,
    HttpRequestReadFailed,
    Skipped,
}

impl From<BatchOperationError> for FileOperationFailure {
    fn from(value: BatchOperationError) -> Self {
        match value {
            BatchOperationError::StorageFailure | BatchOperationError::Communication(_) => {
                Self::StorageFailure
            }
            BatchOperationError::StorageQuotaReached => Self::QuotaReached,
            BatchOperationError::FileNotFound => Self::NotFound,
            BatchOperationError::FileSandboxViolated => Self::SandboxViolated,
            BatchOperationError::FileAlreadyExists => Self::AlreadyExists,
            BatchOperationError::FileNotDirectory => Self::NotDirectory,
            BatchOperationError::FileIsDirectory => Self::IsDirectory,
            BatchOperationError::HttpRequestFailed => Self::HttpRequestFailed,
            BatchOperationError::HttpRequestNotSuccess => Self::HttpRequestNotSuccess,
            BatchOperationError::HttpRequestReadFailed => Self::HttpRequestReadFailed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UrlWriteResult {
    pub path: String,
    pub error: Option<FileOperationFailure>,
}

#[derive(Clone, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct FileOperationResult {
    pub error: Option<FileOperationFailure>,
    pub entry: Option<FileEntry>,
    pub entries: Option<Vec<FileEntry>>,
}
