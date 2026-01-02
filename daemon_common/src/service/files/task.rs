// SPDX-License-Identifier: AGPL-3.0-only

use core::sync::atomic::AtomicU64;
use std::sync::Arc;

#[derive(Clone)]
pub enum StorageTask {
    BackupRestore {
        location: BackupLocation,
        progress: Option<Arc<(AtomicU64, AtomicU64)>>,
    },
    BackupCreate {
        location: Option<BackupLocation>,
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

#[derive(Copy, Clone)]
pub enum FileOperationKind {
    ArchiveCreate,
    ArchiveUnpack,
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

#[derive(Clone)]
pub struct BackupLocation {
    pub storage: String,
    pub id: i64,
}
