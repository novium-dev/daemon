mod fsetstat;
mod fstat;
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

pub use fsetstat::fsetstat;
pub use fstat::fstat;
pub use mkdir::mkdir;
pub use open::{open, open_handle};
pub use read::read;
pub use readdir::readdir;
pub use readlink::readlink;
pub use realpath::realpath;
pub use remove::remove;
pub use rename::rename;
pub use rmdir::rmdir;
pub use setstat::setstat;
pub use stat::stat;
pub use symlink::symlink;
pub use write::write;

use crate::ssh::sftp::{DaemonSftpSession, FileHandle};
use crate::utils::parse_path;
use cap_async_std::fs::*;
use daemon_common::node::BackendAuthServerPermissions;
use russh_sftp::protocol::*;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::debug;

impl DaemonSftpSession {
    pub(crate) fn resolve_path(
        &self,
        mut path: String,
    ) -> Result<
        (
            Option<(Arc<Dir>, BackendAuthServerPermissions)>,
            Vec<String>,
            Vec<String>,
        ),
        StatusCode,
    > {
        debug!("Path0 {}", path);
        // Resolving ..
        let (shorten, ends_with_slash) =
            parse_path(&mut path).map_err(|()| StatusCode::NoSuchFile)?;

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

        if let Some(last) = temp_suffix.last_mut()
            && ends_with_slash
            && last.ends_with('/')
        {
            last.push('/');
        }
        Ok((
            storage.map(|(dir, perms, _)| (Arc::clone(dir), *perms)),
            temp_prefix,
            temp_suffix,
        ))
    }

    pub(crate) fn sandboxed_path(
        &self,
        path: String,
        required: BackendAuthServerPermissions,
    ) -> Result<(Arc<Dir>, PathBuf), StatusCode> {
        let (storage, _, temp_suffix) = self.resolve_path(path)?;

        match storage {
            None => Err(StatusCode::NoSuchFile),
            Some((storage, permissions)) => {
                if !permissions.contains(required) {
                    return Err(StatusCode::PermissionDenied);
                }
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
    pub(crate) const fn has_permissions(&self, required: BackendAuthServerPermissions) -> bool {
        match self {
            Self::Internal(_, _, perms)
            | Self::RegularFile(_, perms)
            | Self::RegularDir(_, _, _, perms) => perms.contains(required),
            Self::Empty => true,
        }
    }
}
