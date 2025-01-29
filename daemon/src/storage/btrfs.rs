use libbtrfs::btrfs::*;
use libbtrfs::btrfs_tree::*;
use libc::ioctl;
use std::collections::HashMap;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::{io, mem};

pub(crate) fn get_subvol_id(file: Result<File, io::Error>) -> std::io::Result<u64> {
    let args = btrfs_ioctl_ino_lookup_args {
        objectid: BTRFS_FIRST_FREE_OBJECTID,
        ..Default::default()
    };
    let file = file?;
    let fd = file.as_raw_fd();

    if unsafe { ioctl(fd, BTRFS_IOC_INO_LOOKUP, &args) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(args.treeid)
}

pub(crate) fn search_quotas(
    file: Result<File, io::Error>,
    out: &mut HashMap<u64, u64>,
) -> Result<(), io::Error> {
    let file = file?;
    let fd = file.as_raw_fd();

    let mut args = btrfs_ioctl_search_args {
        key: btrfs_ioctl_search_key {
            tree_id: BTRFS_QUOTA_TREE_OBJECTID,
            max_type: BTRFS_QGROUP_RELATION_KEY as u32,
            min_type: BTRFS_QGROUP_STATUS_KEY as u32,
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
        if unsafe { ioctl(fd, BTRFS_IOC_TREE_SEARCH, &args) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let sk = &mut args.key;
        if sk.nr_items == 0 {
            break;
        }
        off = 0;
        for _ in 0..sk.nr_items {
            let mut key = btrfs_key {
                typ: 0,
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
            key.typ = sh.typ as u8;
            key.offset = sh.offset;

            if key_in_range(&key, &*sk) {
                match key.typ as usize {
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
                        let rfer_cmpr = info.rfer_cmpr;
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
            sk.min_type = key.typ as u32;
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

fn key_in_range(key: &btrfs_key, sk: &btrfs_ioctl_search_key) -> bool {
    !(key.objectid < sk.min_objectid
        || key.objectid > sk.max_objectid
        || (key.typ as u32) < sk.min_type
        || (key.typ as u32) > sk.max_type
        || key.offset < sk.min_offset
        || key.offset > sk.max_offset)
}
