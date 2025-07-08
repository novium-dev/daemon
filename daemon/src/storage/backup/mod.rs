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
mod create;
mod delete;
mod download;
mod list;
mod name;
mod restore;

use std::io;
use std::os::fd::{AsRawFd, RawFd};
use xattr::FileExt;

pub use create::create_backup;
pub use create::create_backup_order;
pub use delete::delete_backup;
pub use download::backup_to_stream;
pub use download::stream_realtime;
pub use list::list_backups;
pub use name::set_backup_name;
pub use restore::restore_backup;

struct RawFdContainer(RawFd);

impl AsRawFd for RawFdContainer {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl FileExt for RawFdContainer {}

pub trait BackupNameExt {
    /// Cancel safe
    fn get_backup_name(&self) -> impl Future<Output = Result<Option<String>, io::Error>> + Send;

    /// Cancel safe
    fn set_backup_name(
        &self,
        name: Option<String>,
    ) -> impl Future<Output = Result<(), io::Error>> + Send;
}

impl<T: AsRawFd + Sync> BackupNameExt for T {
    async fn get_backup_name(&self) -> Result<Option<String>, io::Error> {
        let file = RawFdContainer(self.as_raw_fd());
        tokio::task::spawn_blocking(move || {
            match file.get_xattr(crate::common::DAEMON_BACKUP_NAME_XATTR) {
                Ok(None) => Ok(None),
                Ok(Some(bytes)) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
                Err(err) if err.raw_os_error() == Some(libc::ENOTSUP) => Ok(None),
                Err(err) => Err(err),
            }
        })
        .await?
    }

    async fn set_backup_name(&self, name: Option<String>) -> Result<(), io::Error> {
        let file = RawFdContainer(self.as_raw_fd());
        tokio::task::spawn_blocking(move || {
            match name {
                Some(name) => {
                    file.set_xattr(crate::common::DAEMON_BACKUP_NAME_XATTR, name.as_bytes())?;
                }
                None => match file.remove_xattr(crate::common::DAEMON_BACKUP_NAME_XATTR) {
                    Ok(()) => {}
                    Err(err) if err.raw_os_error() == Some(libc::ENODATA) => {}
                    Err(err) => return Err(err),
                },
            }
            Ok(())
        })
        .await?
    }
}
