// SPDX-License-Identifier: AGPL-3.0-only
mod create;
mod delete;
pub(crate) mod download;
mod list;
mod metadata;
mod prune;
mod restore;

use async_trait::async_trait;
use axum_extra::response::Attachment;
use daemon_common::service::DataStream;
use daemon_common::service::backup::BackupResponseEntry;
use daemon_common::service::files::task::StorageTask;
use std::collections::HashSet;
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use xattr::FileExt;

pub use create::create_backup;
pub use create::create_backup_order;
pub use delete::delete_backup;
pub use download::backup_to_stream;
pub use download::stream_realtime;
pub use list::list_backups;
pub use metadata::update_backup;
pub use prune::prune_backups;
pub use restore::restore_backup;

use crate::config::StorageEntry;
use crate::storage::StorageIdentifier;
use crate::sync::task::{TaskOwnedRwLockReadGuard, TaskOwnedRwLockWriteGuard};

#[async_trait]
#[expect(clippy::too_many_arguments)]
pub trait BackupBackend {
    fn target_path(&self, id: StorageIdentifier, base: &Path, backup_id: i64) -> PathBuf;

    fn compatible(&self, storage: &StorageEntry) -> bool;

    // Must be cancel safe
    async fn list(
        &self,
        storage: &str,
        id: StorageIdentifier,
        base: &Path,
        entries: &mut Vec<BackupResponseEntry>,
    ) -> Result<(), io::Error>;

    // Must be cancel safe
    async fn create(
        &self,
        id: StorageIdentifier,
        base: &Path,
        backup_id: i64,
        storage: &StorageEntry,
        storage_target: &Path,
        lock: TaskOwnedRwLockReadGuard<StorageTask, ()>,
        exclude: HashSet<String>,
    ) -> Result<TaskOwnedRwLockReadGuard<StorageTask, ()>, io::Error>;

    // Must be cancel safe
    async fn delete(
        &self,
        id: StorageIdentifier,
        base: &Path,
        backup_id: i64,
    ) -> Result<(), io::Error>;

    async fn restore(
        &self,
        id: StorageIdentifier,
        base: &Path,
        backup_id: i64,
        storage: &StorageEntry,
        storage_target: &Path,
        lock: &mut TaskOwnedRwLockWriteGuard<StorageTask, ()>,
        disk: Option<u64>,
        value: Arc<(AtomicI64, AtomicI64)>,
    ) -> Result<(), io::Error>;

    async fn download(
        &self,
        id: StorageIdentifier,
        base: &Path,
        backup_id: i64,
        exclude: HashSet<String>,
    ) -> Result<Attachment<DataStream>, io::Error>;
}

struct RawFdContainer(RawFd);

impl AsRawFd for RawFdContainer {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl FileExt for RawFdContainer {}

pub trait BackupMetadataExt {
    /// Cancel safe
    fn get_backup_name(&self) -> impl Future<Output = Result<Option<String>, io::Error>> + Send;

    /// Cancel safe
    fn set_backup_name(
        &self,
        name: Option<String>,
    ) -> impl Future<Output = Result<(), io::Error>> + Send;

    /// Whether the backup is exempt from retention policies.
    ///
    /// Cancel safe
    fn get_backup_protected(&self) -> impl Future<Output = Result<bool, io::Error>> + Send;

    /// Cancel safe
    fn set_backup_protected(
        &self,
        protected: bool,
    ) -> impl Future<Output = Result<(), io::Error>> + Send;
}

impl<T: AsRawFd + Sync> BackupMetadataExt for T {
    async fn get_backup_name(&self) -> Result<Option<String>, io::Error> {
        get_xattr(self.as_raw_fd(), crate::common::DAEMON_BACKUP_NAME_XATTR)
            .await
            .map(|value| value.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
    }

    async fn set_backup_name(&self, name: Option<String>) -> Result<(), io::Error> {
        set_xattr(
            self.as_raw_fd(),
            crate::common::DAEMON_BACKUP_NAME_XATTR,
            name.map(String::into_bytes),
        )
        .await
    }

    async fn get_backup_protected(&self) -> Result<bool, io::Error> {
        // Only the presence of the attribute matters, its value is never read.
        get_xattr(
            self.as_raw_fd(),
            crate::common::DAEMON_BACKUP_PROTECTED_XATTR,
        )
        .await
        .map(|value| value.is_some())
    }

    async fn set_backup_protected(&self, protected: bool) -> Result<(), io::Error> {
        set_xattr(
            self.as_raw_fd(),
            crate::common::DAEMON_BACKUP_PROTECTED_XATTR,
            protected.then(|| b"1".to_vec()),
        )
        .await
    }
}

/// Reads an extended attribute, treating a filesystem without xattr support as
/// an unset attribute rather than as an error.
///
/// Cancel safe
async fn get_xattr(fd: RawFd, attribute: &'static str) -> Result<Option<Vec<u8>>, io::Error> {
    let file = RawFdContainer(fd);
    tokio::task::spawn_blocking(move || match file.get_xattr(attribute) {
        Ok(value) => Ok(value),
        Err(err) if err.raw_os_error() == Some(libc::ENOTSUP) => Ok(None),
        Err(err) => Err(err),
    })
    .await?
}

/// Cancel safe
async fn set_xattr(
    fd: RawFd,
    attribute: &'static str,
    value: Option<Vec<u8>>,
) -> Result<(), io::Error> {
    let file = RawFdContainer(fd);
    tokio::task::spawn_blocking(move || {
        match value {
            Some(value) => file.set_xattr(attribute, &value)?,
            None => match file.remove_xattr(attribute) {
                Ok(()) => {}
                Err(err) if err.raw_os_error() == Some(libc::ENODATA) => {}
                Err(err) => return Err(err),
            },
        }
        Ok(())
    })
    .await?
}
