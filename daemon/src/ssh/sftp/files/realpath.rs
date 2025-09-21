// SPDX-License-Identifier: AGPL-3.0-only
use crate::ssh::sftp::internal_directory_attributes;
use russh_sftp::protocol::*;
use tracing::debug;

pub async fn realpath(path: String) -> Result<Vec<File>, StatusCode> {
    if path.eq(".") || path.eq("/.") {
        // Required for some sftp clients to get their current path. Otherwise unsupported here
        Ok(vec![File {
            attrs: internal_directory_attributes(),
            longname: String::new(),
            filename: "/".to_owned(),
        }])
    } else {
        debug!("Tried to use realpath on something other than \".\" or \"/.\".");
        Err(StatusCode::PermissionDenied)
    }
}
