use std::{io::ErrorKind, path::PathBuf};

// SPDX-License-Identifier: AGPL-3.0-only
use crate::ssh::sftp::{
    DaemonSftpSession, files::ResolvedPath, internal_directory_attributes, sftp_err,
};
use russh_sftp::protocol::*;
use tracing::{debug, error};

pub async fn realpath(session: &DaemonSftpSession, path: String) -> Result<Vec<File>, StatusCode> {
    let ResolvedPath {
        storage,
        temp_prefix,
        temp_suffix,
    } = session.resolve_path(path)?;
    let mut path = PathBuf::from("/");

    for entry in temp_prefix {
        path = path.join(entry);
    }

    if let Some((dir, _, _)) = storage {
        let mut target_path = if temp_suffix.is_empty() {
            PathBuf::from(".")
        } else {
            PathBuf::new()
        };
        for comp in temp_suffix {
            target_path = target_path.join(comp);
        }
        let target_path = dir
            .canonicalize(target_path)
            .await
            .inspect_err(|err| match err.kind() {
                ErrorKind::NotFound | ErrorKind::PermissionDenied => {
                    debug!("Failed to canonicalize path: {err:?}");
                }
                _ => error!("Failed to canonicalize path: {err:?}"),
            })
            .map_err(sftp_err)?;

        let target_path = target_path.strip_prefix("/").unwrap_or(&target_path);

        path = path.join(target_path);
    }

    Ok(vec![File {
        attrs: internal_directory_attributes(),
        longname: String::new(),
        filename: path.to_string_lossy().to_string(),
    }])
}
