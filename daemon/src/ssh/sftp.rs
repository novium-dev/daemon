use crate::config::StorageEntry;
use crate::ssh::PanelAuthServerPermissions;
use crate::storage::StorageIdentifier;
use crate::utils::parse_path;
use axum::async_trait;
use log::{debug, error};
use russh_sftp::protocol::{
    Attrs, Data, FileAttributes, FileMode, Handle, Name, OpenFlags, Status, StatusCode, Version,
};
use std::collections::HashMap;
use std::fs::{Metadata, Permissions, ReadDir};
use std::io::{ErrorKind, SeekFrom};
use std::os::unix::prelude::*;
use std::path::PathBuf;
use tokio::fs::{self, File, OpenOptions};
use tokio::io;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::OwnedRwLockReadGuard;
use uuid::Uuid;

pub(crate) struct DaemonSftpSession {
    pub(crate) version: Option<u32>,
    pub(crate) file_handles: HashMap<String, FileHandle>,
    pub(crate) access: HashMap<
        Vec<String>,
        (
            StorageIdentifier,
            StorageEntry,
            PanelAuthServerPermissions,
            OwnedRwLockReadGuard<()>,
        ),
    >,
}

#[async_trait]
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
        let handle = self.open(filename, pflags, attrs).await?;
        debug!("Handle: {}", handle);
        Ok(Handle { id, handle })
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        debug!("Realpath {} {}", id, path);
        if path.eq(".") || path.eq("/.") {
            // Required for some sftp clients to get their current path. Otherwise unsupported here
            Ok(Name {
                id,
                files: vec![russh_sftp::protocol::File {
                    attrs: FileAttributes {
                        uid: Some(0),
                        gid: Some(0),
                        size: Some(0),
                        user: Some("root".to_owned()),
                        group: Some("root".to_owned()),
                        permissions: Some(0o755 | FileMode::DIR.bits()),
                        mtime: None,
                        atime: None,
                    },
                    longname: String::new(),
                    filename: "/".to_owned(),
                }],
            })
        } else {
            Err(StatusCode::PermissionDenied)
        }
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        debug!("OpenDir {} {}", id, path);
        let handle = self
            .open(path, OpenFlags::READ, FileAttributes::default())
            .await?;
        debug!("Handle: {}", handle);
        Ok(Handle { id, handle })
    }

    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        debug!("ReadDir {} {}", id, handle);
        let handle = self.handle(handle, PanelAuthServerPermissions::FILES_READ)?;
        let files: Vec<_> = handle
            .readdir()
            .await
            .map_err(|err| {
                if err.kind() == ErrorKind::UnexpectedEof {
                    StatusCode::Eof
                } else {
                    StatusCode::Failure
                }
            })?
            .into_iter()
            .map(|(name, attrs)| russh_sftp::protocol::File::new(name, attrs))
            .collect();
        Ok(Name { id, files })
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        debug!("Read {} {}", id, handle);
        let handle = self
            .handle(handle, PanelAuthServerPermissions::FILES_READ)?
            .regular_only()?;
        handle
            .seek(SeekFrom::Start(offset))
            .await
            .map_err(|err| match err.kind() {
                ErrorKind::UnexpectedEof => StatusCode::Eof,
                _ => StatusCode::Failure,
            })?;
        let mut buf = vec![0; len as usize];

        let read = handle
            .read(&mut buf)
            .await
            .map_err(|err| match err.kind() {
                ErrorKind::UnexpectedEof => StatusCode::Eof,
                _ => StatusCode::Failure,
            })?;
        buf.resize(read, 0u8);
        Ok(Data { id, data: buf })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        debug!("Write {} {}", id, handle);
        let handle = self
            .handle(handle, PanelAuthServerPermissions::FILES_WRITE)?
            .regular_only()?;
        handle
            .seek(SeekFrom::Start(offset))
            .await
            .map_err(|err| match err.kind() {
                ErrorKind::UnexpectedEof => StatusCode::Eof,
                _ => StatusCode::Failure,
            })?;
        handle
            .write_all(&data)
            .await
            .map_err(|err| match err.kind() {
                ErrorKind::UnexpectedEof => StatusCode::Eof,
                _ => StatusCode::Failure,
            })?;
        Self::success(id)
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        debug!("Fstat {} {}", id, handle);
        let handle = self.handle(handle, PanelAuthServerPermissions::FILES_READ)?;
        handle.fstat(id).await
    }

    async fn fsetstat(
        &mut self,
        id: u32,
        handle: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        debug!("Fsetstat {} {}", id, handle);
        let handle = self
            .handle(handle, PanelAuthServerPermissions::FILES_WRITE)?
            .regular_only()?;
        if let Some(mode) = attrs.permissions {
            handle
                .set_permissions(Permissions::from_mode(mode))
                .await
                .map_err(|_err| StatusCode::Failure)?;
        }

        if let Some(len) = attrs.size {
            handle
                .set_len(len)
                .await
                .map_err(|_err| StatusCode::Failure)?;
        }
        Self::success(id)
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        debug!("Lstat {} {}", id, path);
        let path = self
            .path(path, PanelAuthServerPermissions::FILES_READ)
            .await?;
        let metadata = tokio::fs::symlink_metadata(path)
            .await
            .map_err(|_err| StatusCode::Failure)?;
        Ok(Attrs {
            id,
            attrs: metadata.into_fileattributes(),
        })
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        debug!("Mkdir {} {}", id, path);
        let path = self
            .path(path, PanelAuthServerPermissions::FILES_CREATE)
            .await?;
        fs::create_dir(&path)
            .await
            .map_err(|_err| StatusCode::Failure)?;
        if let Some(mode) = attrs.permissions {
            fs::set_permissions(&path, Permissions::from_mode(mode))
                .await
                .map_err(|_err| StatusCode::Failure)?;
        }
        Self::success(id)
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        debug!("Stat {} {}", id, path);
        let path = self
            .path(path, PanelAuthServerPermissions::FILES_READ)
            .await?;
        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|_err| StatusCode::Failure)?;
        Ok(Attrs {
            id,
            attrs: metadata.into_fileattributes(),
        })
    }

    async fn setstat(
        &mut self,
        id: u32,
        path: String,
        attrs: FileAttributes,
    ) -> Result<Status, Self::Error> {
        debug!("Setstat {} {}", id, path);
        let path = self
            .path(path, PanelAuthServerPermissions::FILES_WRITE)
            .await?;
        let fs = fs::File::open(path)
            .await
            .map_err(|_err| StatusCode::Failure)?;
        if let Some(mode) = attrs.permissions {
            fs.set_permissions(Permissions::from_mode(mode))
                .await
                .map_err(|_err| StatusCode::Failure)?;
        }

        if let Some(len) = attrs.size {
            fs.set_len(len).await.map_err(|_err| StatusCode::Failure)?;
        }

        Self::success(id)
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        debug!("Rename {} {} to {}", id, oldpath, newpath);
        let old_path = self
            .path(oldpath, PanelAuthServerPermissions::FILES_DELETE)
            .await?;
        let new_path = self
            .path(newpath, PanelAuthServerPermissions::FILES_CREATE)
            .await?;
        fs::rename(old_path, new_path)
            .await
            .map_err(|_err| StatusCode::Failure)?;
        Self::success(id)
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, Self::Error> {
        debug!("Remove {} {}", id, filename);
        let path = self
            .path(filename, PanelAuthServerPermissions::FILES_DELETE)
            .await?;
        fs::remove_file(path)
            .await
            .map_err(|_err| StatusCode::Failure)?;
        Self::success(id)
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        debug!("rmdir {} {}", id, path);
        let path = self
            .path(path, PanelAuthServerPermissions::FILES_DELETE)
            .await?;
        fs::remove_dir(path)
            .await
            .map_err(|_err| StatusCode::Failure)?;
        Self::success(id)
    }

    async fn symlink(
        &mut self,
        id: u32,
        linkpath: String,
        targetpath: String,
    ) -> Result<Status, Self::Error> {
        debug!("Symlink {} {} {}", id, linkpath, targetpath);
        let linkpath = self
            .path(linkpath, PanelAuthServerPermissions::FILES_CREATE)
            .await?;
        let targetpath = PathBuf::from(targetpath);
        fs::symlink(linkpath, targetpath)
            .await
            .map_err(|_err| StatusCode::Failure)?;
        Self::success(id)
    }

    async fn readlink(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        debug!("Readlink {} {}", id, path);
        let linkpath = self
            .path(path, PanelAuthServerPermissions::FILES_READ)
            .await?;
        let path = fs::read_link(linkpath)
            .await
            .map_err(|_err| StatusCode::Failure)?;
        Ok(Name {
            id,
            files: vec![russh_sftp::protocol::File {
                filename: path.to_string_lossy().into_owned(),
                longname: "".to_string(),
                attrs: FileAttributes::default(),
            }],
        })
    }
}

impl DaemonSftpSession {
    async fn path0(
        &self,
        mut path: String,
    ) -> Result<
        (
            Option<(StorageIdentifier, &StorageEntry, PanelAuthServerPermissions)>,
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
            storage.map(|(id, storage, perms, _)| (*id, storage, *perms)),
            temp_prefix,
            temp_suffix,
        ))
    }

    async fn path(
        &self,
        path: String,
        required: PanelAuthServerPermissions,
    ) -> Result<PathBuf, StatusCode> {
        let (storage, _, temp_suffix) = self.path0(path).await?;

        match storage {
            None => Err(StatusCode::NoSuchFile),
            Some((id, storage, permissions)) => {
                if !permissions.contains(required) {
                    return Err(StatusCode::PermissionDenied);
                }
                let mut path = PathBuf::from(storage.path()).join(id.as_pathbuf());
                for comp in temp_suffix {
                    path = path.join(comp);
                }
                Ok(path)
            }
        }
    }

    fn handle(
        &mut self,
        handle: String,
        permissions: PanelAuthServerPermissions,
    ) -> Result<&mut FileHandle, StatusCode> {
        match self.file_handles.get_mut(&handle) {
            None => Err(StatusCode::NoSuchFile),
            Some(handle) => {
                if !handle.permissions(permissions) {
                    return Err(StatusCode::PermissionDenied);
                }
                Ok(handle)
            }
        }
    }

    fn success(id: u32) -> Result<Status, StatusCode> {
        Ok(Status {
            id,
            error_message: "Ok".to_owned(),
            status_code: StatusCode::Ok,
            language_tag: "en-US".to_owned(),
        })
    }

    async fn open(
        &mut self,
        path: String,
        pflags: OpenFlags,
        attrs: FileAttributes,
    ) -> Result<String, StatusCode> {
        let (storage, temp_prefix, temp_suffix) = self.path0(path).await?;
        match storage {
            None => {
                let mut entries = Vec::new();
                for entry in self.access.keys() {
                    if temp_prefix.len() >= entry.len() {
                        continue;
                    }
                    let mut allowed = true;
                    for (i, item) in temp_prefix.iter().enumerate() {
                        if !entry[i].eq(item) {
                            allowed = false;
                            break;
                        }
                    }
                    if allowed {
                        entries.push(entry[temp_prefix.len()].clone());
                    }
                }
                if entries.is_empty() {
                    return if temp_prefix.is_empty() {
                        let handle = FileHandle::Internal(
                            Vec::new(),
                            false,
                            PanelAuthServerPermissions::all(),
                        );
                        let empty = "empty".to_string();
                        self.file_handles.insert(empty.clone(), handle);
                        Ok(empty)
                    } else {
                        Err(StatusCode::NoSuchFile)
                    };
                }
                let entries: Vec<_> = entries
                    .into_iter()
                    .map(|name| {
                        (
                            name,
                            FileAttributes {
                                uid: Some(0),
                                gid: Some(0),
                                size: Some(0),
                                user: Some("root".to_owned()),
                                group: Some("root".to_owned()),
                                permissions: Some(0o755 | FileMode::DIR.bits()),
                                mtime: None,
                                atime: None,
                            },
                        )
                    })
                    .collect();
                let handle = format!("internal_{}", Uuid::new_v4());
                self.file_handles.insert(
                    handle.clone(),
                    FileHandle::Internal(entries, false, PanelAuthServerPermissions::all()),
                );
                Ok(handle)
            }
            Some((id, storage, permissions)) => {
                let base = PathBuf::from(storage.path()).join(id.as_pathbuf());
                let base = tokio::fs::canonicalize(base)
                    .await
                    .map_err(|_err| StatusCode::Failure)?;

                let mut path = base.clone();
                for comp in temp_suffix {
                    path = path.join(comp);
                }

                let cononical_path = tokio::fs::canonicalize(&path)
                    .await
                    .map_err(|_err| StatusCode::Failure)?;
                if cononical_path.strip_prefix(base).is_err() {
                    return Err(StatusCode::PermissionDenied); // Trying to escape the allowed directory using symlinks
                }

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

                let file = options.open(&path).await.map_err(|err| match err.kind() {
                    ErrorKind::NotFound => StatusCode::NoSuchFile,
                    ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
                    _ => StatusCode::Failure,
                })?;
                let mut read_dir = None;
                if file
                    .metadata()
                    .await
                    .map_err(|err| match err.kind() {
                        ErrorKind::NotFound => StatusCode::NoSuchFile,
                        ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
                        _ => StatusCode::Failure,
                    })?
                    .is_dir()
                {
                    read_dir = Some(
                        tokio::task::spawn_blocking(move || std::fs::read_dir(path))
                            .await
                            .map_err(|_err| StatusCode::Failure)?
                            .map_err(|err| match err.kind() {
                                ErrorKind::NotFound => StatusCode::NoSuchFile,
                                ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
                                _ => StatusCode::Failure,
                            })?,
                    );
                }
                let handle = file.as_raw_fd().to_string();
                self.file_handles.insert(
                    handle.clone(),
                    FileHandle::Regular(file, read_dir, false, permissions),
                );
                Ok(handle)
            }
        }
    }
}

pub(crate) enum FileHandle {
    Regular(File, Option<ReadDir>, bool, PanelAuthServerPermissions),
    Internal(
        Vec<(String, FileAttributes)>,
        bool,
        PanelAuthServerPermissions,
    ),
}

impl FileHandle {
    fn permissions(&self, required: PanelAuthServerPermissions) -> bool {
        match self {
            FileHandle::Internal(_, _, perms) | FileHandle::Regular(_, _, _, perms) => {
                perms.contains(required)
            }
        }
    }

    async fn fstat(&mut self, id: u32) -> Result<Attrs, StatusCode> {
        match self {
            FileHandle::Regular(handle, _, _, _) => {
                let metadata = handle
                    .metadata()
                    .await
                    .map_err(|_err| StatusCode::Failure)?;
                Ok(Attrs {
                    id,
                    attrs: metadata.into_fileattributes(),
                })
            }
            FileHandle::Internal(_, _, _) => Ok(Attrs {
                id,
                attrs: FileAttributes {
                    permissions: Some(0o755 | FileMode::DIR.bits()),
                    uid: Some(0),
                    gid: Some(0),
                    user: Some("root".to_owned()),
                    group: Some("root".to_owned()),
                    size: Some(0),
                    mtime: None,
                    atime: None,
                },
            }),
        }
    }

    async fn readdir(&mut self) -> Result<Vec<(String, FileAttributes)>, io::Error> {
        match self {
            FileHandle::Regular(_handle, read_dir, read, _) => {
                if read_dir.is_none() {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        "File is not a directory",
                    ));
                }
                if *read {
                    return Err(io::Error::new(ErrorKind::UnexpectedEof, ""));
                }
                *read = true;
                let mut read_dir_taken = read_dir.take().unwrap();
                let (read_dir_taken, result) = tokio::task::spawn_blocking(move || {
                    let mut files = Vec::new();
                    loop {
                        match read_dir_taken.next() {
                            Some(Ok(entry)) => {
                                let name = entry.file_name().to_string_lossy().into_owned();
                                let metadata = entry.metadata();
                                match metadata {
                                    Ok(metadata) => {
                                        files.push((name, metadata.into_fileattributes()));
                                    }
                                    Err(err) => break (read_dir_taken, Err(err)),
                                }
                            }
                            Some(Err(err)) => break (read_dir_taken, Err(err)),
                            None => {
                                break (read_dir_taken, Ok(files));
                            }
                        }
                    }
                })
                .await
                .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                *read_dir = Some(read_dir_taken);
                result
            }
            FileHandle::Internal(dirs, read, _) => {
                if *read {
                    return Err(io::Error::new(ErrorKind::UnexpectedEof, ""));
                }
                *read = true;
                Ok(dirs.clone())
            }
        }
    }

    fn regular_only(&mut self) -> Result<&mut File, StatusCode> {
        match self {
            FileHandle::Regular(file, _, _, _) => Ok(file),
            FileHandle::Internal(_, _, _) => Err(StatusCode::PermissionDenied),
        }
    }
}

pub(crate) trait IntoFileAttributes {
    fn into_fileattributes(self) -> FileAttributes;
}

impl IntoFileAttributes for Metadata {
    fn into_fileattributes(self) -> FileAttributes {
        let mut r#type = 0;
        let file_type = self.file_type();
        if file_type.is_dir() {
            r#type |= FileMode::DIR.bits();
        } else if file_type.is_symlink() {
            r#type |= FileMode::LNK.bits();
        } else if file_type.is_socket() {
            r#type |= FileMode::SOCK.bits();
        } else if file_type.is_block_device() {
            r#type |= FileMode::BLK.bits();
        } else if file_type.is_char_device() {
            r#type |= FileMode::CHR.bits();
        } else if file_type.is_fifo() {
            r#type |= FileMode::FIFO.bits();
        } else {
            r#type |= FileMode::REG.bits();
        }
        FileAttributes {
            permissions: Some(self.mode() | r#type),
            size: Some(self.size()),
            uid: Some(self.uid()),
            gid: Some(self.gid()),
            atime: self.atime().try_into().map(Some).unwrap_or(None),
            mtime: self.mtime().try_into().map(Some).unwrap_or(None),
            user: None,
            group: None,
        }
    }
}
