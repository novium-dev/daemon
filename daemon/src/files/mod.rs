// SPDX-License-Identifier: AGPL-3.0-only
mod archive;
mod copy;
mod delete;
mod download;
mod list;
mod mkdir;
mod rename;
mod stat;
mod symlink;
mod watch;
mod wipe;
mod write;

use crate::config::StorageEntry;
use crate::storage::sync::StorageTokens;
use crate::storage::{STORAGE_TOKEN_MAGIC, StorageIdentifier};
use crate::utils::path_relative;
pub use archive::{create_archive, unpack_archive};
use cap_async_std::ambient_authority;
use cap_async_std::fs::{Dir, FileTypeExt as _, Metadata, MetadataExt as _};
pub use copy::copy_file;
use core::str::FromStr as _;
use daemon_common::files::FileMode;
use daemon_common::service::files::FileEntry;
pub use delete::delete_file;
pub use download::download_file;
pub use list::list_files;
pub use mkdir::mkdir_file;
pub use rename::rename_file;
pub use stat::stat_file;
use std::io;
use std::io::ErrorKind;
use std::path::PathBuf;
pub use symlink::symlink_file;
use tracing::error;
use uuid::Uuid;
pub use watch::watch_files;
pub use wipe::wipe;
pub use write::{write_file, write_file_by_template, write_file_by_url};

async fn storage_path_root(
    id: StorageIdentifier,
    storage: &StorageEntry,
) -> Result<(Dir, PathBuf), io::Error> {
    let base = PathBuf::from(storage.path()).join(id.as_pathbuf());
    Ok((
        Dir::open_ambient_dir(&base, ambient_authority())
            .await
            .inspect_err(|err| error!("Failed to open storage {:?}: {:?}", id, err))?,
        base,
    ))
}

async fn storage_path(
    id: StorageIdentifier,
    storage: &StorageEntry,
    mut path: String,
    tokens: &StorageTokens,
) -> Result<(Dir, PathBuf, PathBuf), io::Error> {
    let base = PathBuf::from(storage.path()).join(id.as_pathbuf());

    let dir = if let Some(path_stripped) = path.strip_prefix(STORAGE_TOKEN_MAGIC)
        && let Some(index) = path_stripped.find(';')
    {
        if index != 36 {
            return Err(io::Error::new(
                ErrorKind::NotFound,
                "found token magic, but first ; was at wrong location",
            ));
        }
        let Some(id) = path_stripped.get(0..36) else {
            return Err(io::Error::new(
                ErrorKind::NotFound,
                "found token magic, but was unable to parse uuid (utf8 issue)",
            ));
        };
        match Uuid::from_str(id) {
            Ok(id) => {
                let dir = tokens.get(&id).await.ok_or_else(|| {
                    io::Error::new(
                        ErrorKind::NotFound,
                        "found token magic, but id was not found",
                    )
                })?;
                if let Some(path_stripped) = path_stripped.get(36..) {
                    path = path_stripped.to_owned();
                } else {
                    path = String::new();
                }

                Dir::clone(&dir)
            }
            Err(_) => {
                return Err(io::Error::new(
                    ErrorKind::NotFound,
                    "found token magic, but was unable to parse uuid",
                ));
            }
        }
    } else {
        Dir::open_ambient_dir(&base, ambient_authority())
            .await
            .inspect_err(|err| error!("Failed to open storage {id:?}: {err:?}"))?
    };

    Ok((dir, base, path_relative(&path)))
}

pub trait IntoFileEntry {
    fn into_fileentry(
        self,
        filename: String,
        relative_path: String,
        path: Option<String>,
        archive: bool,
    ) -> FileEntry;
}

impl IntoFileEntry for Metadata {
    fn into_fileentry(
        self,
        filename: String,
        relative_path: String,
        path: Option<String>,
        archive: bool,
    ) -> FileEntry {
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
        FileEntry {
            name: filename,
            path,
            mode: self.mode() | r#type,
            relative_path,
            size: self.size(),
            uid: self.uid(),
            gid: self.gid(),
            archive,
        }
    }
}
