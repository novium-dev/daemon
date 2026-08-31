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

use crate::utils;
use libc::ioctl;
use std::collections::HashMap;
use std::io;
use std::io::ErrorKind;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use sys::*;
use tokio::fs::File;

pub fn get_subvol_id(file: Result<impl AsRawFd, io::Error>) -> io::Result<u64> {
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

pub fn search_quotas(
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
pub async fn apply_qgroup_limit(dir: &PathBuf, disk: Option<u64>) -> Result<(), io::Error> {
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
    let file = File::open(&dir).await?;

    // SAFETY: ioctl syscall, should always be safe
    let ret = tokio::task::spawn_blocking(move || unsafe {
        ioctl(file.as_raw_fd(), BTRFS_IOC_QGROUP_LIMIT as _, &args)
    })
    .await
    .map_err(io::Error::other)?;
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
pub async fn new_subvolume(dir: &Path, subvolname: &str) -> io::Result<()> {
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
pub async fn remove_subvolume(dir: &Path, subvolname: &str) -> io::Result<()> {
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

pub async fn new_snapshot(
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
