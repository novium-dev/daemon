/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
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
