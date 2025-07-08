mod archive;
mod copy;
mod delete;
mod download;
mod list;
mod mkdir;
mod rename;
mod stat;
mod symlink;
mod write;

use std::io;
use std::path::PathBuf;

use crate::config::StorageEntry;
use crate::storage::StorageIdentifier;
use cap_async_std::ambient_authority;
use cap_async_std::fs::{Dir, FileTypeExt as _, Metadata, MetadataExt as _};
use daemon_common::files::FileMode;
use daemon_common::service::files::FileEntry;
use tracing::error;

pub use archive::{create_archive, unpack_archive};
pub use copy::copy_file;
pub use delete::delete_file;
pub use download::download_file;
pub use list::list_files;
pub use mkdir::mkdir_file;
pub use rename::rename_file;
pub use stat::stat_file;
pub use symlink::symlink_file;
pub use write::{write_file, write_file_by_template, write_file_by_url};

async fn storage_path(
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
