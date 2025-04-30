mod create;
mod delete;
mod download;
mod list;
mod name;
mod restore;

use std::io;
use std::os::fd::{AsRawFd, RawFd};
use xattr::FileExt;

pub(crate) use create::create_backup;
pub(crate) use create::create_backup_order;
pub(crate) use delete::delete_backup;
pub(crate) use download::backup_to_stream;
pub(crate) use download::stream_realtime;
pub(crate) use list::list_backups;
pub(crate) use name::set_backup_name;
pub(crate) use restore::restore_backup;

struct RawFdContainer(RawFd);

impl AsRawFd for RawFdContainer {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl FileExt for RawFdContainer {}

pub(crate) trait BackupNameExtension {
    async fn get_backup_name(&self) -> Result<Option<String>, io::Error>;

    async fn set_backup_name(&self, name: Option<String>) -> Result<(), io::Error>;
}

impl<T: AsRawFd> BackupNameExtension for T {
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
                    file.set_xattr(crate::common::DAEMON_BACKUP_NAME_XATTR, name.as_bytes())?
                }
                None => match file.remove_xattr(crate::common::DAEMON_BACKUP_NAME_XATTR) {
                    Ok(_) => {}
                    Err(err) if err.raw_os_error() == Some(libc::ENODATA) => {}
                    Err(err) => return Err(err),
                },
            }
            Ok(())
        })
        .await?
    }
}
