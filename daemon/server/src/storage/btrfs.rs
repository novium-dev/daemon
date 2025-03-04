mod sys;

use crate::utils;
use libc::ioctl;
use std::collections::HashMap;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::{io, mem};
use sys::*;
use tokio::fs::File;

pub(crate) fn get_subvol_id(file: Result<impl AsRawFd, io::Error>) -> std::io::Result<u64> {
    let args = btrfs_ioctl_ino_lookup_args {
        objectid: BTRFS_FIRST_FREE_OBJECTID as _,
        ..Default::default()
    };
    let file = file?;
    let fd = file.as_raw_fd();

    if unsafe { ioctl(fd, BTRFS_IOC_INO_LOOKUP as _, &args) } < 0 {
        return Err(std::io::Error::last_os_error());
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
            tree_id: BTRFS_QUOTA_TREE_OBJECTID as _,
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

    let mut off: u64;

    'l: loop {
        if unsafe { ioctl(fd, BTRFS_IOC_TREE_SEARCH as _, &args) } < 0 {
            return Err(std::io::Error::last_os_error());
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
            let mut buf = [0; size_of::<btrfs_ioctl_search_header>()];
            buf.copy_from_slice(
                &args.buf[off as usize..(off as usize + size_of::<btrfs_ioctl_search_header>())],
            );
            let sh: btrfs_ioctl_search_header = unsafe { mem::transmute(buf) };

            off += size_of::<btrfs_ioctl_search_header>() as u64;

            key.objectid = sh.objectid;
            key.type_ = sh.type_ as u8;
            key.offset = sh.offset;

            if key_in_range(&key, &*sk) {
                match key.type_ as u32 {
                    BTRFS_QGROUP_STATUS_KEY
                    | BTRFS_QGROUP_LIMIT_KEY
                    | BTRFS_QGROUP_RELATION_KEY => {}
                    BTRFS_QGROUP_INFO_KEY => {
                        let mut buf = [0; size_of::<btrfs_qgroup_info_item>()];
                        buf.copy_from_slice(
                            &args.buf[off as usize
                                ..(off as usize + size_of::<btrfs_qgroup_info_item>())],
                        );
                        let info: btrfs_qgroup_info_item = unsafe { mem::transmute(buf) };
                        let rfer_cmpr = info.referenced_compressed;
                        let qgroupid = key.offset;

                        let level = (qgroupid >> BTRFS_QGROUP_LEVEL_SHIFT) as u16;
                        let subvolid = qgroupid & ((1u64 << BTRFS_QGROUP_LEVEL_SHIFT) - 1);

                        if level == 0 {
                            out.insert(subvolid, rfer_cmpr);
                        }
                    }
                    _ => break 'l,
                }
            }

            off += sh.len as u64;

            sk.min_objectid = key.objectid;
            sk.min_type = key.type_ as u32;
            sk.min_offset = key.offset;
        }
        sk.nr_items = 4096;
        if sk.min_offset < u64::MAX {
            sk.min_offset += 1;
        } else {
            break;
        }
    }
    Ok(())
}

pub(crate) async fn apply_qgroup_limit(dir: &PathBuf, disk: Option<u64>) -> Result<(), io::Error> {
    use std::io::ErrorKind;

    let args = btrfs_ioctl_qgroup_limit_args {
        qgroupid: 0,
        lim: btrfs_qgroup_limit {
            flags: (BTRFS_QGROUP_LIMIT_RFER_CMPR
                | BTRFS_QGROUP_LIMIT_EXCL_CMPR
                | BTRFS_QGROUP_LIMIT_MAX_RFER) as u64,
            max_referenced: disk.map(utils::from_mib).unwrap_or(u64::MAX), // This is the value for no limit

            rsv_exclusive: 0,
            rsv_referenced: 0,
            max_exclusive: 0,
        },
    };
    let file = File::open(&dir).await?;

    let ret = tokio::task::spawn_blocking(move || unsafe {
        libc::ioctl(file.as_raw_fd(), BTRFS_IOC_QGROUP_LIMIT as _, &args)
    })
    .await
    .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn key_in_range(key: &btrfs_key, sk: &btrfs_ioctl_search_key) -> bool {
    !(key.objectid < sk.min_objectid
        || key.objectid > sk.max_objectid
        || (key.type_ as u32) < sk.min_type
        || (key.type_ as u32) > sk.max_type
        || key.offset < sk.min_offset
        || key.offset > sk.max_offset)
}

pub async fn new_subvolume(dir: &Path, subvolname: &str) -> io::Result<()> {
    let fd = tokio::fs::File::open(dir).await?;

    let mut ret = [0; 4040usize];
    ret.iter_mut().take(4039).enumerate().for_each(|(i, item)| {
        if let Some(ch) = subvolname.chars().nth(i) {
            *item = ch as i8
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

        if unsafe { ioctl(fd.as_raw_fd(), BTRFS_IOC_SUBVOL_CREATE_V2 as _, &args) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })
    .await?
}

pub async fn remove_subvolume(dir: &Path, subvolname: &str) -> io::Result<()> {
    let fd = tokio::fs::File::open(dir).await?;

    let mut ret = [0; 4040usize];
    ret.iter_mut().take(4039).enumerate().for_each(|(i, item)| {
        if let Some(ch) = subvolname.chars().nth(i) {
            *item = ch as i8
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

        if unsafe { ioctl(fd.as_raw_fd(), BTRFS_IOC_SNAP_DESTROY_V2 as _, &args) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })
    .await?
}

pub async fn new_snapshot(
    subvolpath: &Path,
    dir: &Path,
    snapshotname: &str,
    readonly: bool,
) -> io::Result<()> {
    let fd = tokio::fs::File::open(dir).await?;
    let subvol_fd = tokio::fs::File::open(subvolpath).await?;

    let mut ret = [0; 4040usize];
    ret.iter_mut().take(4039).enumerate().for_each(|(i, item)| {
        if let Some(ch) = snapshotname.chars().nth(i) {
            *item = ch as i8
        }
    });

    tokio::task::spawn_blocking(move || {
        let args = btrfs_ioctl_vol_args_v2 {
            __bindgen_anon_2: btrfs_ioctl_vol_args_v2__bindgen_ty_2 { name: ret },
            __bindgen_anon_1: btrfs_ioctl_vol_args_v2__bindgen_ty_1 { unused: [0; 4] },
            fd: subvol_fd.as_raw_fd() as _,
            transid: 0,
            flags: if readonly {
                BTRFS_SUBVOL_RDONLY as _
            } else {
                0
            },
        };

        if unsafe { ioctl(fd.as_raw_fd(), BTRFS_IOC_SNAP_CREATE_V2 as _, &args) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })
    .await?
}
