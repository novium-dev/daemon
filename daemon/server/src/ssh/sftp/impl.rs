use crate::ssh::sftp::{DaemonSftpSession, FileHandle};
use cap_async_std::fs::{OpenOptions, OpenOptionsExt};
use daemon_common::server::BackendAuthServerPermissions;
use indexmap::IndexMap;
use russh_sftp::protocol::*;
use std::collections::HashMap;
use tracing::*;

impl DaemonSftpSession {
    fn success(id: u32) -> Result<Status, StatusCode> {
        Ok(Status {
            id,
            error_message: "Ok".to_owned(),
            status_code: StatusCode::Ok,
            language_tag: "en-US".to_owned(),
        })
    }
}

impl russh_sftp::server::Handler for DaemonSftpSession {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn init(
        &mut self,
        version: u32,
        extensions: HashMap<String, String>,
    ) -> Result<Version, Self::Error> {
        if self.version.is_some() {
            error!("duplicate SSH_FXP_VERSION packet");
            return Err(StatusCode::ConnectionLost);
        }

        self.version = Some(version);

        let handle =
            FileHandle::Internal(IndexMap::new(), false, BackendAuthServerPermissions::all());
        let empty = "empty".to_string();
        self.file_handles.insert(empty.clone(), handle);

        debug!("version: {:?}, extensions: {:?}", self.version, extensions);
        Ok(Version::new())
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        debug!("Close {} {}", id, handle);
        match self.file_handles.remove(&handle) {
            None => Err(StatusCode::NoSuchFile),
            Some(_handle) => Self::success(id),
        }
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        attrs: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        debug!("Open {} {} {}", id, filename, pflags.bits());

        let mut options = OpenOptions::new();
        let mut options = options
            .read(pflags.intersects(OpenFlags::READ))
            .write(pflags.intersects(OpenFlags::WRITE))
            .append(pflags.intersects(OpenFlags::APPEND))
            .create(pflags.intersects(OpenFlags::CREATE))
            .truncate(pflags.intersects(OpenFlags::TRUNCATE))
            .create_new(pflags.intersects(OpenFlags::EXCLUDE));

        if let Some(mode) = attrs.permissions {
            options = options.mode(mode);
        }

        let handle = super::files::open(self, filename, options).await?;
        debug!("Handle: {}", handle);
        Ok(Handle { id, handle })
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        debug!("Realpath {} {}", id, path);
        super::files::realpath(path)
            .await
            .map(|files| Name { id, files })
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        debug!("OpenDir {} {}", id, path);
        let handle = super::files::open(self, path, &OpenOptions::new()).await?;
        debug!("Handle: {}", handle);
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        debug!("ReadDir {} {}", id, handle);
        let handle = self.handle(handle, BackendAuthServerPermissions::FILES_READ)?;
        Ok(Name {
            id,
            files: super::files::readdir(handle).await?,
        })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        debug!("Read {} {}", id, handle);
        let handle = self.handle(handle, BackendAuthServerPermissions::FILES_READ)?;
        super::files::read(handle, offset, len)
            .await
            .map(|buf| Data { id, data: buf })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        debug!("Write {} {}", id, handle);
        let handle = self.handle(handle, BackendAuthServerPermissions::FILES_WRITE)?;
        super::files::write(handle, offset, data).await?;
        Self::success(id)
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        debug!("Fstat {} {}", id, handle);
        let handle = self.handle(handle, BackendAuthServerPermissions::FILES_READ)?;
        super::files::fstat(handle)
            .await
            .map(|attrs| Attrs { id, attrs })
    }

    async fn fsetstat(
        &mut self,
        id: u32,
        handle: String,
        attrs: FileAttributes,
    ) -> Result<Attrs, Self::Error> {
        debug!("Fsetstat {} {}", id, handle);
        let handle = self.handle(handle, BackendAuthServerPermissions::FILES_WRITE)?;
        super::files::fsetstat(handle, attrs.permissions, attrs.size)
            .await
            .map(|attrs| Attrs { id, attrs })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        debug!("Lstat {} {}", id, path);
        let handle = super::files::open(self, path, OpenOptions::new().read(true)).await?;
        let mut handle = self.file_handles.remove(&handle).unwrap();
        super::files::fstat(&mut handle)
            .await
            .map(|attrs| Attrs { id, attrs })
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        debug!("Mkdir {} {}", id, path);
        let (storage, path) = self
            .sandboxed_path(path, BackendAuthServerPermissions::FILES_WRITE)
            .await?;
        super::files::mkdir(storage, path, attrs.permissions).await?;
        Self::success(id)
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        debug!("Stat {} {}", id, path);
        let (storage, path) = self
            .sandboxed_path(path, BackendAuthServerPermissions::FILES_READ)
            .await?;
        super::files::stat(storage, path)
            .await
            .map(|attrs| Attrs { id, attrs })
    }

    async fn setstat(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        debug!("Setstat {} {}", id, path);
        let (storage, path) = self
            .sandboxed_path(path, BackendAuthServerPermissions::FILES_WRITE)
            .await?;
        super::files::setstat(storage, path, attrs.permissions, attrs.size).await?;
        Self::success(id)
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        debug!("Rename {} {} to {}", id, oldpath, newpath);
        let (old_storage, old_path) = self
            .sandboxed_path(oldpath, BackendAuthServerPermissions::FILES_WRITE)
            .await?;
        let (new_storage, new_path) = self
            .sandboxed_path(newpath, BackendAuthServerPermissions::FILES_WRITE)
            .await?;
        super::files::rename(old_storage, old_path, new_storage, new_path).await?;
        Self::success(id)
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        debug!("Remove {} {}", id, filename);
        let (storage, path) = self
            .sandboxed_path(filename, BackendAuthServerPermissions::FILES_WRITE)
            .await?;
        super::files::remove(storage, path).await?;
        Self::success(id)
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        debug!("rmdir {} {}", id, path);
        let (storage, path) = self
            .sandboxed_path(path, BackendAuthServerPermissions::FILES_WRITE)
            .await?;
        super::files::rmdir(storage, path).await?;
        Self::success(id)
    }

    async fn symlink(
        &mut self,
        id: u32,
        linkpath: String,
        targetpath: String,
    ) -> Result<Status, Self::Error> {
        debug!("Symlink {} {} {}", id, linkpath, targetpath);
        let (storage, path) = self
            .sandboxed_path(linkpath, BackendAuthServerPermissions::FILES_WRITE)
            .await?;
        super::files::symlink(storage, path, targetpath).await?;
        Self::success(id)
    }

    async fn readlink(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        debug!("Readlink {} {}", id, path);
        let (storage, path) = self
            .sandboxed_path(path, BackendAuthServerPermissions::FILES_READ)
            .await?;
        super::files::readlink(storage, path)
            .await
            .map(|filename| Name {
                id,
                files: vec![File {
                    filename,
                    longname: "".to_string(),
                    attrs: FileAttributes::default(),
                }],
            })
    }
}
