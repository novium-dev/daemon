mod fsetstat;
mod fstat;
mod lstat;
mod mkdir;
mod open;
mod read;
mod readdir;
mod readlink;
mod realpath;
mod remove;
mod rename;
mod rmdir;
mod setstat;
mod stat;
mod symlink;
mod write;

pub(crate) use fsetstat::fsetstat;
pub(crate) use fstat::fstat;
pub(crate) use lstat::lstat;
pub(crate) use mkdir::mkdir;
pub(crate) use open::open;
pub(crate) use read::read;
pub(crate) use readdir::readdir;
pub(crate) use readlink::readlink;
pub(crate) use realpath::realpath;
pub(crate) use remove::remove;
pub(crate) use rename::rename;
pub(crate) use rmdir::rmdir;
pub(crate) use setstat::setstat;
pub(crate) use stat::stat;
pub(crate) use symlink::symlink;
pub(crate) use write::write;

use crate::config::StorageEntry;
use crate::ssh::sftp::{sftp_err, DaemonSftpSession, FileHandle};
use crate::storage::StorageIdentifier;
use crate::utils::parse_path;
use cap_async_std::ambient_authority;
use cap_async_std::fs::*;
use daemon_common::server::BackendAuthServerPermissions;
use log::debug;
use russh_sftp::protocol::*;
use std::path::PathBuf;

impl DaemonSftpSession {
    pub(crate) async fn resolve_path(
        &self,
        mut path: String,
    ) -> Result<
        (
            Option<(
                StorageIdentifier,
                StorageEntry,
                BackendAuthServerPermissions,
            )>,
            Vec<String>,
            Vec<String>,
        ),
        StatusCode,
    > {
        debug!("Path0 {}", path);
        // Resolving ..
        let (shorten, ends_with_slash) =
            parse_path(&mut path).map_err(|_| StatusCode::NoSuchFile)?;

        let mut temp_prefix = Vec::new();
        let mut temp_suffix = Vec::new();
        let mut storage = None;
        for comp in shorten {
            match storage {
                None => {
                    temp_prefix.push(comp.to_owned());
                    storage = self.access.get(&temp_prefix);
                }
                Some(_storage) => {
                    temp_suffix.push(comp.to_owned());
                }
            }
        }

        if !temp_suffix.is_empty()
            && ends_with_slash
            && !temp_suffix[temp_suffix.len() - 1].ends_with('/')
        {
            let index = temp_suffix.len() - 1;
            temp_suffix[index] = format!("{}/", temp_suffix[index]);
        }
        Ok((
            storage.map(|(id, storage, perms, _)| (*id, storage.clone(), *perms)),
            temp_prefix,
            temp_suffix,
        ))
    }

    pub(crate) async fn sandboxed_path(
        &self,
        path: String,
        required: BackendAuthServerPermissions,
    ) -> Result<(Dir, PathBuf), StatusCode> {
        let (storage, _, temp_suffix) = self.resolve_path(path).await?;

        match storage {
            None => Err(StatusCode::NoSuchFile),
            Some((id, storage, permissions)) => {
                if !permissions.contains(required) {
                    return Err(StatusCode::PermissionDenied);
                }
                let base = PathBuf::from(storage.path()).join(id.as_pathbuf());
                let storage = Dir::open_ambient_dir(base, ambient_authority())
                    .await
                    .map_err(sftp_err)?;
                let mut path = PathBuf::new();
                for comp in temp_suffix {
                    path = path.join(comp);
                }
                Ok((storage, path))
            }
        }
    }
}

impl FileHandle {
    pub(crate) fn has_permissions(&self, required: BackendAuthServerPermissions) -> bool {
        match self {
            FileHandle::Internal(_, _, perms)
            | FileHandle::RegularFile(_, perms)
            | FileHandle::RegularDir(_, _, _, perms) => perms.contains(required),
        }
    }
}
