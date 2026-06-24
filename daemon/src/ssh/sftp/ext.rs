// SPDX-License-Identifier: AGPL-3.0-only
use cap_async_std::fs::{FileTypeExt as _, Metadata, MetadataExt as _};
use russh_sftp::protocol::{FileAttributes, FileMode};

pub trait IntoFileAttributes {
    fn into_fileattributes(self) -> FileAttributes;
}

impl IntoFileAttributes for std::fs::Metadata {
    fn into_fileattributes(self) -> FileAttributes {
        Metadata::from_just_metadata(self).into_fileattributes()
    }
}

impl IntoFileAttributes for Metadata {
    fn into_fileattributes(self) -> FileAttributes {
        let mut r#type = 0;
        let file_type = self.file_type();
        if file_type.is_dir() {
            r#type |= FileMode::DIR.bits();
        } else if file_type.is_symlink() {
            r#type |= FileMode::LNK.bits();
        } else if file_type.is_socket() {
            r#type |= FileMode::SOCK.bits();
        } else if file_type.is_block_device() {
            r#type |= FileMode::BLK.bits();
        } else if file_type.is_char_device() {
            r#type |= FileMode::CHR.bits();
        } else if file_type.is_fifo() {
            r#type |= FileMode::FIFO.bits();
        } else {
            r#type |= FileMode::REG.bits();
        }
        FileAttributes {
            permissions: Some(self.mode() | r#type),
            size: Some(self.size()),
            uid: Some(self.uid()),
            gid: Some(self.gid()),
            atime: self.atime().try_into().ok(),
            mtime: self.mtime().try_into().ok(),
            user: None,
            group: None,
        }
    }
}
