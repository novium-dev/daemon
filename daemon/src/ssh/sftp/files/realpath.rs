use crate::ssh::sftp::internal_directory_attributes;
use russh_sftp::protocol::*;

pub(crate) async fn realpath(path: String) -> Result<Vec<File>, StatusCode> {
    if path.eq(".") || path.eq("/.") {
        // Required for some sftp clients to get their current path. Otherwise unsupported here
        Ok(vec![File {
            attrs: internal_directory_attributes(),
            longname: String::new(),
            filename: "/".to_owned(),
        }])
    } else {
        Err(StatusCode::PermissionDenied)
    }
}
