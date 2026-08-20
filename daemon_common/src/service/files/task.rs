// SPDX-License-Identifier: AGPL-3.0-only

use core::sync::atomic::AtomicU64;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum StorageTask {
    BackupRestore {
        location: BackupLocation,
        #[schema(value_type = Option<AtomicProgress>)]
        progress: Option<Arc<AtomicProgress>>,
    },
    BackupCreate {
        location: Option<BackupLocation>,
        #[schema(value_type = Option<u64>)]
        progress: Option<Arc<AtomicU64>>,
    },
    BackupDownload {
        location: BackupLocation,
    },
    BackupDelete {
        location: BackupLocation,
    },
    BackupList,
    BackupUpdate {
        location: BackupLocation,
    },
    RealtimeDownload,
    Wipe,
    Sftp,
    FilesWatchWebsocket,
    FileOperation {
        kind: FileOperationKind,
    },
    Scan,
    ServerRunning,
}

#[derive(Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AtomicProgress {
    #[schema(value_type = u64)]
    pub current: AtomicU64,
    #[schema(value_type = u64)]
    pub max: AtomicU64,
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum FileOperationKind {
    ArchiveCreate,
    ArchiveUnpack,
    Chmod,
    CopyFile,
    Delete { recursive: bool },
    Download,
    List,
    Mkdir { recursive: bool },
    Rename,
    Stat,
    Symlink,
    Write,
    WriteByUrl,
    WriteByTemplate,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BackupLocation {
    pub storage: String,
    pub id: i64,
}
