// SPDX-License-Identifier: AGPL-3.0-only
use crate::ssh::sftp::sftp_err;
use cap_async_std::fs::Dir;
use russh_sftp::protocol::StatusCode;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error};

pub async fn readlink(storage: Arc<Dir>, path: PathBuf) -> Result<String, StatusCode> {
    storage
        .read_link(path)
        .await
        .inspect_err(|err| match err.kind() {
            ErrorKind::NotFound | ErrorKind::PermissionDenied | ErrorKind::InvalidData => {
                debug!("Failed to readlink: {err:?}");
            }
            _ => error!("Failed to readlink: {err:?}"),
        })
        .map_err(sftp_err)
        .map(|buf| buf.to_string_lossy().to_string())
}
