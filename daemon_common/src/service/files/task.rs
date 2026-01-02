// SPDX-License-Identifier: AGPL-3.0-only

use core::sync::atomic::AtomicU64;

pub enum StorageTask {
    BackupRestore {
        id: u64,
        progress: Option<(AtomicU64, AtomicU64)>,
    },
    BackupCreate {
        id: u64,
        progress: Option<AtomicU64>,
    },
    Sftp,
    FilesWatchWebsocket,
    FileOperation,
}
