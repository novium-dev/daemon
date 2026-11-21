// SPDX-License-Identifier: AGPL-3.0-only

#![allow(
    clippy::cast_lossless,
    reason = "ioctl can differ on different platform, for example u64 on glibc and i32 on musl"
)]
#![allow(
    clippy::cast_possible_wrap,
    reason = "ioctl can differ on different platform, for example u64 on glibc and i32 on musl"
)]
pub(crate) mod sys;

use crate::config::{ArchiveCompression, StorageBackendEntry, StorageEntry};
use crate::storage::archive::format::ArchiveFormat;
use crate::storage::backup::download::{backup_filename, stream_realtime1};
use crate::storage::backup::{BackupBackend, BackupMetadataExt as _};
use crate::storage::worker::{ApplyQuotaToken, ResolvedStorage, apply_quotas};
use crate::storage::{StorageBackend, StorageIdentifier};
use crate::sync::task::{TaskOwnedRwLockReadGuard, TaskOwnedRwLockWriteGuard};
use crate::utils::{self, read_dir_opt};
use async_trait::async_trait;
use axum_extra::response::Attachment;
use chrono::{TimeZone as _, Utc};
use daemon_common::service::DataStream;
use daemon_common::service::backup::BackupResponseEntry;
use daemon_common::service::files::task::StorageTask;
use libc::ioctl;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::File as StdFile;
use std::io;
use std::io::ErrorKind;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use sys::*;
use tokio::fs::File;
use tracing::{debug, error};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BtrfsStorageBackend {
    pub group: String,
    pub quota: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BtrfsBackupBackend {
    pub group: String,
    pub quota: bool,
    pub jit_compression: ArchiveCompression,
}

#[async_trait]
impl StorageBackend for BtrfsStorageBackend {
    async fn create(
        &self,
        _id: StorageIdentifier,
        _base: &Path,
        target: &Path,
    ) -> Result<(), io::Error> {
        let Some(parent) = target.parent() else {
            error!("Got path without parent: {}", target.display());
            return Err(io::Error::other("Invalid path: No parent"));
        };
        tokio::fs::create_dir_all(parent).await.inspect_err(|err| {
            error!("Unable to create parents of storage directory: {err:?}");
        })?;
        let Some(filename) = target.file_name() else {
            error!("Missing final component in storage path");
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "No final component in storage path",
            ));
        };
        let Some(filename) = filename.to_str() else {
            error!("Final component in storage path is not utf8");
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "Final component in storage path is not utf8",
            ));
        };
        new_subvolume(parent, filename).await.inspect_err(|err| {
            error!("Unable to create subvolume of storage directory: {err:?}");
        })?;
        Ok(())
    }

    async fn apply_quota(
        &self,
        _id: StorageIdentifier,
        _base: &Path,
        target: &Path,
        limit: Option<u64>,
    ) -> Result<bool, io::Error> {
        if self.quota {
            apply_qgroup_limit(target, limit).await?;
        }
        Ok(self.quota)
    }

    async fn delete(
        &self,
        _id: StorageIdentifier,
        _base: &Path,
        target: &Path,
    ) -> Result<(), io::Error> {
        let Some(parent) = target.parent() else {
            error!("Got path without parent: {}", target.display());
            return Err(io::Error::other("Invalid path: No parent"));
        };
        let Some(filename) = target.file_name() else {
            error!("Missing final component in storage path");
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "No final component in storage path",
            ));
        };
        let Some(filename) = filename.to_str() else {
            error!("Final component in storage path is not utf8");
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "Final component in storage path is not utf8",
            ));
        };
        remove_subvolume(parent, filename)
            .await
            .inspect_err(|err| {
                error!("Unable to remove subvolume of storage directory: {err:?}");
            })?;
        Ok(())
    }

    fn quota_usage<'a>(
        &self,
        base: &Path,
        storages: &'a HashMap<StorageIdentifier, ResolvedStorage>,
    ) -> Result<Option<ApplyQuotaToken<'a>>, io::Error> {
        if !self.quota {
            return Ok(None);
        }

        let mut quotas = HashMap::new();
        super::btrfs::search_quotas(StdFile::open(base), &mut quotas)?;

        Ok(Some(apply_quotas(storages, |id, storage| {
            let subvol = super::btrfs::get_subvol_id(StdFile::open(&storage.path))?;
            quotas.get(&subvol).copied().ok_or_else(|| {
                io::Error::other(format!("No btrfs quota found: {id:?} (subvolid {subvol})"))
            })
        })))
    }
}

#[async_trait]
impl BackupBackend for BtrfsBackupBackend {
    fn target_path(&self, id: StorageIdentifier, base: &Path, backup_id: i64) -> PathBuf {
        let target_dir = base.join(id.as_pathbuf());
        target_dir.join(backup_id.to_string())
    }

    fn compatible(&self, storage: &StorageEntry) -> bool {
        matches!(storage,
            StorageEntry {
                backend: StorageBackendEntry::Btrfs { backend: BtrfsStorageBackend { group, .. } },
                 ..
            }
            if group.eq(&self.group)
        )
    }

    async fn list(
        &self,
        storage: &str,
        id: StorageIdentifier,
        base: &Path,
        entries: &mut Vec<BackupResponseEntry>,
    ) -> Result<(), io::Error> {
        let target_dir = base.join(id.as_pathbuf());
        let Some(mut readdir) = read_dir_opt(&target_dir)
            .await
            .inspect_err(|err| error!("Failed to list backups (read_dir): {err:?}"))?
        else {
            return Ok(());
        };

        let mut quotas = HashMap::new();
        if self.quota {
            quotas = tokio::task::spawn_blocking(move || {
                if let Err(err) = search_quotas(std::fs::File::open(target_dir), &mut quotas) {
                    error!("Unable to retrieve quotas: {err:?}. Ensure quotas are enabled");
                }
                quotas
            })
            .await
            .inspect_err(|err| {
                error!("Failed to list backups (retrieve quotas): {err:?}");
            })
            .map_err(io::Error::from)?;
        }
        while let Some(entry) = readdir
            .next_entry()
            .await
            .inspect_err(|err| error!("Failed to list backups (next_entry): {err:?}"))?
        {
            let metadata = entry.metadata().await.inspect_err(|err| {
                error!("Failed to list backups (metadata): {err:?}");
            })?;
            if !metadata.file_type().is_dir() {
                continue;
            }
            let entry_path = entry.path();
            let entry = entry.file_name().to_string_lossy().to_string();
            let entry: Result<i64, _> = entry.parse();

            let file = File::open(&entry_path)
                .await
                .inspect_err(|err| error!("Failed to list backups (open): {err:?}"))?;
            let name = file.get_backup_name().await.inspect_err(|err| {
                error!("Failed to list backups (get_name): {err:?}");
            })?;
            let protected = file.get_backup_protected().await.inspect_err(|err| {
                error!("Failed to list backups (get_protected): {err:?}");
            })?;
            let size = if self.quota {
                match tokio::task::spawn_blocking(move || get_subvol_id(Ok(file)))
                    .await
                    .inspect_err(|err| {
                        error!("Failed to list backups (get_subvol_id): {err:?}");
                    })
                    .map_err(io::Error::from)?
                {
                    Ok(subvol) => quotas.get(&subvol).copied(),
                    Err(err) => {
                        error!(
                            "Unable to retrieve subvolume id: {err:?} in {:?}. Ensure this backup is a subvolume and not a directory!",
                            entry_path.to_string_lossy(),
                        );
                        None
                    }
                }
            } else {
                None
            };

            if let Ok(entry) = entry {
                entries.push(BackupResponseEntry {
                    time: entry,
                    storage: storage.to_owned(),
                    size,
                    name,
                    protected,
                });
            }
        }
        Ok(())
    }

    async fn create(
        &self,
        id: StorageIdentifier,
        base: &Path,
        backup_id: i64,
        _storage: &StorageEntry,
        storage_target: &Path,
        lock: TaskOwnedRwLockReadGuard<StorageTask, ()>,
        _exclude: HashSet<String>,
    ) -> Result<TaskOwnedRwLockReadGuard<StorageTask, ()>, io::Error> {
        let target_dir = base.join(id.as_pathbuf());

        if !tokio::fs::try_exists(&target_dir)
            .await
            .inspect_err(|err| error!("Backup creation failed (try_exists): {:?}", err))?
        {
            tokio::fs::create_dir_all(&target_dir)
                .await
                .inspect_err(|err| error!("Backup creation failed (create_dir_all): {:?}", err))?;
        }

        let subvol_fd = tokio::fs::File::open(storage_target)
            .await
            .inspect_err(|err| error!("Backup creation failed (open): {err:?}"))?;

        new_snapshot(subvol_fd, &target_dir, &backup_id.to_string(), true)
            .await
            .inspect_err(|err| error!("Backup creation failed (snapshot): {err:?}"))?;
        Ok(lock)
    }

    async fn delete(
        &self,
        id: StorageIdentifier,
        base: &Path,
        backup_id: i64,
    ) -> Result<(), io::Error> {
        let target_dir = base.join(id.as_pathbuf());
        remove_subvolume(&target_dir, &backup_id.to_string())
            .await
            .inspect_err(|err| {
                if err.kind() == ErrorKind::NotFound {
                    debug!("Failed to delete a backup (delete_subvolume): {err:?}");
                } else {
                    error!("Failed to delete a backup (delete_subvolume): {err:?}");
                }
            })
    }

    async fn restore(
        &self,
        id: StorageIdentifier,
        base: &Path,
        backup_id: i64,
        _storage: &StorageEntry,
        storage_target: &Path,
        _lock: &mut TaskOwnedRwLockWriteGuard<StorageTask, ()>,
        _disk: Option<u64>,
        _value: Arc<(AtomicI64, AtomicI64)>,
    ) -> Result<(), io::Error> {
        let target = self.target_path(id, base, backup_id);
        let parent = storage_target.join("..");
        let fd = tokio::fs::File::open(&target).await.inspect_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                debug!("Failed to restore backup (open): {err:?}");
            } else {
                error!("Failed to restore backup (open): {err:?}");
            }
        })?;
        let Some(filename) = storage_target.file_name() else {
            error!("Missing final component in storage path");
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "No final component in storage path",
            ));
        };
        let Some(filename) = filename.to_str() else {
            error!("Final component in storage path is not utf8");
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "Final component in storage path is not utf8",
            ));
        };
        remove_subvolume(&parent, filename)
            .await
            .inspect_err(|err| error!("Failed to restore backup (delete_snapshot): {err:?}"))?;
        new_snapshot(fd, &parent, filename, false)
            .await
            .inspect_err(|err| error!("Failed to restore backup (create_snapshot): {err:?}"))?;
        Ok(())
    }

    async fn download(
        &self,
        id: StorageIdentifier,
        base: &Path,
        backup_id: i64,
        exclude: HashSet<String>,
    ) -> Result<Attachment<DataStream>, io::Error> {
        let target = self.target_path(id, base, backup_id);
        Ok(
            Attachment::new(stream_realtime1(self.jit_compression, target, exclude).await)
                .content_type(ArchiveFormat::Tar(self.jit_compression).content_type())
                .filename(format!(
                    "{}_{}.tar{}",
                    backup_filename(id),
                    Utc.timestamp_millis_opt(backup_id).unwrap().to_rfc3339(),
                    self.jit_compression.ext()
                )),
        )
    }
}

pub(crate) fn get_subvol_id(file: Result<impl AsRawFd, io::Error>) -> io::Result<u64> {
    let mut args = btrfs_ioctl_ino_lookup_args {
        treeid: 0,
        objectid: BTRFS_FIRST_FREE_OBJECTID.into(),
        ..Default::default()
    };
    let file = file?;
    let fd = file.as_raw_fd();

    // SAFETY: ioctl syscall, should always be safe
    if unsafe { ioctl(fd, BTRFS_IOC_INO_LOOKUP as _, &mut args) } < 0i32 {
        return Err(io::Error::last_os_error());
    }
    Ok(args.treeid)
}

pub(crate) fn search_quotas(
    file: Result<impl AsRawFd, io::Error>,
    out: &mut HashMap<u64, u64>,
) -> Result<(), io::Error> {
    let file = file?;
    let fd = file.as_raw_fd();

    let mut args = btrfs_ioctl_search_args {
        key: btrfs_ioctl_search_key {
            tree_id: BTRFS_QUOTA_TREE_OBJECTID.into(),
            max_type: BTRFS_QGROUP_RELATION_KEY,
            min_type: BTRFS_QGROUP_STATUS_KEY,
            max_objectid: u64::MAX,
            max_offset: u64::MAX,
            max_transid: u64::MAX,
            nr_items: 4096,

            ..Default::default()
        },
        ..btrfs_ioctl_search_args::default()
    };

    let mut off: usize;

    'l: loop {
        // SAFETY: ioctl syscall, should always be safe
        if unsafe { ioctl(fd, BTRFS_IOC_TREE_SEARCH as _, &mut args) } < 0i32 {
            return Err(io::Error::last_os_error());
        }
        let sk = &mut args.key;
        if sk.nr_items == 0 {
            break;
        }
        off = 0;
        for _ in 0..sk.nr_items {
            let mut key = btrfs_key {
                type_: 0,
                objectid: 0,
                offset: 0,
            };
            let Some(sh) = args.buf.get_struct::<btrfs_ioctl_search_header>(off) else {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "Buffer out of range",
                ));
            };

            off = off
                .checked_add(size_of::<btrfs_ioctl_search_header>())
                .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "Offset overflowed"))?;

            key.objectid = sh.objectid;
            key.type_ = sh.type_ as u8;
            key.offset = sh.offset;

            if key_in_range(&key, &*sk) {
                match u32::from(key.type_) {
                    BTRFS_QGROUP_STATUS_KEY
                    | BTRFS_QGROUP_LIMIT_KEY
                    | BTRFS_QGROUP_RELATION_KEY => {}
                    BTRFS_QGROUP_INFO_KEY => {
                        let Some(info) = args.buf.get_struct::<btrfs_qgroup_info_item>(off) else {
                            return Err(io::Error::new(
                                ErrorKind::InvalidData,
                                "Buffer out of range",
                            ));
                        };
                        let rfer_cmpr = info.referenced_compressed;
                        let qgroupid = key.offset;

                        let level = (qgroupid >> BTRFS_QGROUP_LEVEL_SHIFT) & 0xFFFF;
                        let subvolid = qgroupid & ((1u64 << BTRFS_QGROUP_LEVEL_SHIFT) - 1);

                        if level == 0u64 {
                            out.insert(subvolid, rfer_cmpr);
                        }
                    }
                    _ => break 'l,
                }
            }

            off = off
                .checked_add(usize::try_from(sh.len).map_err(io::Error::other)?)
                .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "Offset overflowed"))?;

            sk.min_objectid = key.objectid;
            sk.min_type = u32::from(key.type_);
            sk.min_offset = key.offset;
        }
        sk.nr_items = 4096u32;
        if let Some(min_offset) = sk.min_offset.checked_add(1u64) {
            sk.min_offset = min_offset;
        } else {
            break;
        }
    }
    Ok(())
}

/// Cancel safe
async fn apply_qgroup_limit(target: &Path, disk: Option<u64>) -> Result<(), io::Error> {
    let args = btrfs_ioctl_qgroup_limit_args {
        qgroupid: 0,
        lim: btrfs_qgroup_limit {
            flags: u64::from(
                BTRFS_QGROUP_LIMIT_RFER_CMPR
                    | BTRFS_QGROUP_LIMIT_EXCL_CMPR
                    | BTRFS_QGROUP_LIMIT_MAX_RFER,
            ),
            max_referenced: disk.map_or(u64::MAX, utils::from_mib), // This is the value for no limit

            rsv_exclusive: 0,
            rsv_referenced: 0,
            max_exclusive: 0,
        },
    };
    let file = File::open(target).await?;

    // SAFETY: ioctl syscall, should always be safe
    let ret = tokio::task::spawn_blocking(move || unsafe {
        ioctl(file.as_raw_fd(), BTRFS_IOC_QGROUP_LIMIT as _, &args)
    })
    .await
    .map_err(io::Error::from)?;
    if ret < 0i32 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn key_in_range(key: &btrfs_key, sk: &btrfs_ioctl_search_key) -> bool {
    !(key.objectid < sk.min_objectid
        || key.objectid > sk.max_objectid
        || u32::from(key.type_) < sk.min_type
        || u32::from(key.type_) > sk.max_type
        || key.offset < sk.min_offset
        || key.offset > sk.max_offset)
}

/// Cancel safe
async fn new_subvolume(dir: &Path, subvolname: &str) -> io::Result<()> {
    let fd = tokio::fs::File::open(dir).await?;

    let mut ret = [0; 4040usize];
    ret.iter_mut().take(4039).enumerate().for_each(|(i, item)| {
        if let Some(ch) = subvolname.chars().nth(i) {
            *item = ch as i8;
        }
    });

    tokio::task::spawn_blocking(move || {
        let args = btrfs_ioctl_vol_args_v2 {
            __bindgen_anon_2: btrfs_ioctl_vol_args_v2__bindgen_ty_2 { name: ret },
            __bindgen_anon_1: btrfs_ioctl_vol_args_v2__bindgen_ty_1 { unused: [0; 4] },
            fd: 0,
            transid: 0,
            flags: 0,
        };

        // SAFETY: ioctl syscall, should always be safe
        if unsafe { ioctl(fd.as_raw_fd(), BTRFS_IOC_SUBVOL_CREATE_V2 as _, &args) } < 0i32 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })
    .await?
}

/// Cancel safe
async fn remove_subvolume(dir: &Path, subvolname: &str) -> io::Result<()> {
    let fd = tokio::fs::File::open(dir).await?;

    let mut ret = [0; 4040usize];
    ret.iter_mut().take(4039).enumerate().for_each(|(i, item)| {
        if let Some(ch) = subvolname.chars().nth(i) {
            *item = ch as i8;
        }
    });

    tokio::task::spawn_blocking(move || {
        let args = btrfs_ioctl_vol_args_v2 {
            __bindgen_anon_2: btrfs_ioctl_vol_args_v2__bindgen_ty_2 { name: ret },
            __bindgen_anon_1: btrfs_ioctl_vol_args_v2__bindgen_ty_1 { unused: [0; 4] },
            fd: 0,
            transid: 0,
            flags: 0,
        };

        // SAFETY: ioctl syscall, should always be safe
        if unsafe { ioctl(fd.as_raw_fd(), BTRFS_IOC_SNAP_DESTROY_V2 as _, &args) } < 0i32 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })
    .await?
}

async fn new_snapshot(
    subvol_fd: tokio::fs::File,
    dir: &Path,
    snapshotname: &str,
    readonly: bool,
) -> io::Result<()> {
    let fd = tokio::fs::File::open(dir).await?;

    let mut ret = [0; 4040usize];
    ret.iter_mut().take(4039).enumerate().for_each(|(i, item)| {
        if let Some(ch) = snapshotname.chars().nth(i) {
            *item = ch as i8;
        }
    });

    tokio::task::spawn_blocking(move || {
        let args = btrfs_ioctl_vol_args_v2 {
            __bindgen_anon_2: btrfs_ioctl_vol_args_v2__bindgen_ty_2 { name: ret },
            __bindgen_anon_1: btrfs_ioctl_vol_args_v2__bindgen_ty_1 { unused: [0; 4] },
            fd: subvol_fd.as_raw_fd().into(),
            transid: 0,
            flags: if readonly {
                BTRFS_SUBVOL_RDONLY.into()
            } else {
                0
            },
        };

        // SAFETY: ioctl syscall, should always be safe
        if unsafe { ioctl(fd.as_raw_fd(), BTRFS_IOC_SNAP_CREATE_V2 as _, &args) } < 0i32 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })
    .await?
}

// Helper methods

trait BufferExt {
    fn get_struct<T: Zeroable>(&self, offset: usize) -> Option<&T>;
}

impl BufferExt for [i8] {
    fn get_struct<T: Zeroable>(&self, offset: usize) -> Option<&T> {
        let ptr =
            core::ptr::from_ref(self.get(offset..offset.checked_add(size_of::<T>())?)?).cast::<T>();
        // SAFETY: If T implements Zeroable, it doesn't contain any pointers and is therefore safe
        Some(unsafe { &*ptr })
    }
}

/**
 *  # SAFETY
 *  All implementations should not contain any pointers.
 */
unsafe trait Zeroable: Send + Sync + 'static {}

// SAFETY: Doesn't contain pointers, so its safe to implement
unsafe impl Zeroable for btrfs_qgroup_info_item {}

// SAFETY: Doesn't contain pointers, so its safe to implement
unsafe impl Zeroable for btrfs_ioctl_search_header {}
