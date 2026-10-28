// SPDX-License-Identifier: AGPL-3.0-only
//! ext4 project quotas.
//!
//! Every server storage directory is assigned its own project (the server id)
//! with the inherit flag set, so everything created below it is accounted to
//! that project by the kernel. Usage is read from the quota instead of
//! scanning the directory, and the limit is enforced by the kernel.
//!
//! Requires a filesystem created with `-O quota,project` (or enabled later with
//! `tune2fs -O quota,project`) and mounted with `prjquota`.

#![allow(
    clippy::cast_possible_wrap,
    reason = "ioctl can differ on different platform, for example u64 on glibc and i32 on musl"
)]
#![allow(
    clippy::cast_lossless,
    reason = "ioctl can differ on different platform, for example u64 on glibc and i32 on musl"
)]

use crate::utils;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::{FileTypeExt as _, OpenOptionsExt as _};
use std::path::Path;

const PRJQUOTA: libc::c_int = 2;
/// Unit of `dqb_bhardlimit` and `dqb_bsoftlimit`.
const QIF_DQBLKSIZE: u64 = 1024;
const FS_IOC_FSGETXATTR: u32 = 0x801C_581F;
const FS_IOC_FSSETXATTR: u32 = 0x401C_5820;
const FS_XFLAG_PROJINHERIT: u32 = 0x0000_0200;

#[repr(C)]
#[derive(Default)]
struct FsXAttr {
    xflags: u32,
    extsize: u32,
    nextents: u32,
    projid: u32,
    cowextsize: u32,
    pad: [u8; 8],
}

fn open(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
}

fn get_xattr(file: &File) -> io::Result<FsXAttr> {
    let mut attr = FsXAttr::default();
    // SAFETY: attr is a valid fsxattr, which FS_IOC_FSGETXATTR writes to
    if unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            FS_IOC_FSGETXATTR as _,
            core::ptr::from_mut(&mut attr),
        )
    } < 0i32
    {
        return Err(io::Error::last_os_error());
    }
    Ok(attr)
}

fn set_project(file: &File, project: u32, inherit: bool) -> io::Result<()> {
    let mut attr = get_xattr(file)?;
    let flags = if inherit {
        attr.xflags | FS_XFLAG_PROJINHERIT
    } else {
        attr.xflags
    };
    if attr.projid == project && attr.xflags == flags {
        return Ok(());
    }
    attr.projid = project;
    attr.xflags = flags;
    // SAFETY: attr is a valid fsxattr, which FS_IOC_FSSETXATTR reads from
    if unsafe {
        libc::ioctl(
            file.as_raw_fd(),
            FS_IOC_FSSETXATTR as _,
            core::ptr::from_ref(&attr),
        )
    } < 0i32
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn quotactl(
    file: &File,
    cmd: libc::c_int,
    project: u32,
    quota: &mut libc::dqblk,
) -> io::Result<()> {
    // SAFETY: quota is a valid dqblk, which Q_GETQUOTA and Q_SETQUOTA expect
    if unsafe {
        libc::syscall(
            libc::SYS_quotactl_fd,
            file.as_raw_fd(),
            libc::QCMD(cmd, PRJQUOTA),
            project,
            core::ptr::from_mut(quota),
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

const fn empty_quota() -> libc::dqblk {
    libc::dqblk {
        dqb_bhardlimit: 0,
        dqb_bsoftlimit: 0,
        dqb_curspace: 0,
        dqb_ihardlimit: 0,
        dqb_isoftlimit: 0,
        dqb_curinodes: 0,
        dqb_btime: 0,
        dqb_itime: 0,
        dqb_valid: 0,
    }
}

/// Assigns `project` to `dir` and everything below it.
///
/// Only walks the directory if `dir` itself is not assigned yet, for example
/// when an existing directory storage is switched to ext4. The directory
/// itself is assigned last, so an interrupted walk is retried.
pub fn assign_project(dir: &Path, project: u32) -> io::Result<()> {
    let root = open(dir)?;
    let attr = get_xattr(&root)?;
    if attr.projid == project && attr.xflags & FS_XFLAG_PROJINHERIT != 0 {
        return Ok(());
    }

    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let entries = match fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err),
        };
        for entry in entries {
            let entry = entry?;
            let file_type = entry.file_type()?;
            // Symlinks and special files cannot be opened to assign them, but
            // they are tiny and anything created later inherits the project.
            if file_type.is_symlink()
                || file_type.is_fifo()
                || file_type.is_socket()
                || file_type.is_block_device()
                || file_type.is_char_device()
            {
                continue;
            }
            let path = entry.path();
            match open(&path) {
                Ok(file) => set_project(&file, project, file_type.is_dir())?,
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err),
            }
            if file_type.is_dir() {
                pending.push(path);
            }
        }
    }

    set_project(&root, project, true)
}

/// Sets the space limit of `project`. `None` removes the limit.
///
/// `path` may be any directory on the filesystem.
pub fn apply_limit(path: &Path, project: u32, disk: Option<u64>) -> io::Result<()> {
    let limit = disk.map_or(0, |mib| utils::from_mib(mib).div_ceil(QIF_DQBLKSIZE));
    let mut quota = libc::dqblk {
        dqb_bhardlimit: limit,
        dqb_bsoftlimit: limit,
        dqb_valid: libc::QIF_BLIMITS,
        ..empty_quota()
    };
    quotactl(&open(path)?, libc::Q_SETQUOTA, project, &mut quota)
}

/// Returns the bytes used by `project`.
///
/// `fs` may be any opened directory on the filesystem.
pub fn usage(fs: &File, project: u32) -> io::Result<u64> {
    let mut quota = empty_quota();
    quotactl(fs, libc::Q_GETQUOTA, project, &mut quota)?;
    Ok(quota.dqb_curspace)
}

/// Verifies that `path` is on ext4 with project quotas enabled.
pub fn check(path: &Path) -> Result<(), String> {
    let file = open(path).map_err(|err| format!("Unable to open: {err}"))?;
    // SAFETY: zeroed is a valid statfs, which fstatfs fills
    let mut stat: libc::statfs = unsafe { core::mem::zeroed() };
    // SAFETY: stat is a valid statfs
    if unsafe { libc::fstatfs(file.as_raw_fd(), &raw mut stat) } < 0i32 {
        return Err(format!("statfs failed: {}", io::Error::last_os_error()));
    }
    if i128::from(stat.f_type) != i128::from(libc::EXT4_SUPER_MAGIC) {
        return Err("the path is not on an ext4 filesystem".to_owned());
    }
    usage(&file, 0)
        .map(|_usage| ())
        .map_err(|err| format!("project quotas are not enabled (mount with prjquota): {err}"))
}

#[cfg(test)]
mod tests {
    use super::{apply_limit, assign_project, open, usage};
    use std::fs;
    use std::io::Write as _;
    use std::path::PathBuf;

    /// Needs `EXT4_QUOTA_TEST_DIR` pointing to a directory on an ext4 filesystem
    /// mounted with `prjquota`, and `CAP_SYS_ADMIN` without `CAP_SYS_RESOURCE`
    /// (which bypasses quota limits).
    #[test]
    #[ignore = "requires an ext4 filesystem with project quotas"]
    fn enforces_project_quota() {
        let base = PathBuf::from(std::env::var("EXT4_QUOTA_TEST_DIR").expect("test dir"));
        let dir = base.join("quota-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("existing")).expect("create dirs");
        fs::write(dir.join("existing/file"), vec![1u8; 64 * 1024]).expect("write existing");

        let project = 4242;
        assign_project(&dir, project).expect("assign project");
        apply_limit(&dir, project, Some(1)).expect("apply limit");
        let fs_root = open(&base).expect("open base");
        assert!(usage(&fs_root, project).expect("usage") >= 64 * 1024);

        let mut file = fs::File::create(dir.join("new")).expect("create file");
        let result = file
            .write_all(&vec![1u8; 2 * 1024 * 1024])
            .and_then(|()| file.sync_all());
        assert!(result.is_err(), "limit not enforced");

        drop(file);
        fs::remove_dir_all(&dir).expect("cleanup");
        apply_limit(&base, project, None).expect("remove limit");
        assert_eq!(usage(&fs_root, project).expect("usage"), 0);
    }
}
