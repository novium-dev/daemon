use crate::files::{
    copy_file, create_archive, delete_file, download_file, list_files, mkdir_file, rename_file,
    stat_file, symlink_file, unpack_archive, write_file, write_file_by_template, write_file_by_url,
};
use crate::storage::StorageIdentifier;
use crate::{Daemon, DaemonExtra};
use async_trait::async_trait;
use axum_extra::TypedHeader;
use axum_extra::headers::ContentLength;
use daemon_common::service::files::FileEntry;
use daemon_common::service::files::FilesService;
use daemon_common::service::files::error::{
    ServerCopyFileError, ServerCreateArchiveError, ServerCreateDirectoryError,
    ServerDeleteFileError, ServerDownloadFileError, ServerDownloadFileUserError,
    ServerListFilesError, ServerRenameFileError, ServerStatFileError, ServerSymlinkError,
    ServerUnpackArchiveError, ServerUploadFileError, ServerUploadFileUserError,
    ServerWriteFileByTemplateError, ServerWriteFileByUrlError,
};
use daemon_common::service::server::ServerFileWrite;
use daemon_common::service::{DataStream, FileResponse};
use daemon_common::user::UserSignature;
use url::Url;

#[async_trait]
impl<T: DaemonExtra> FilesService for Daemon<T> {
    async fn server_list_files(
        &self,
        server_id: u32,
        path: String,
    ) -> Result<Vec<FileEntry>, ServerListFilesError> {
        let storage = StorageIdentifier::Primary(server_id);
        list_files(self, storage, path).await
    }

    async fn server_stat_file(
        &self,
        server_id: u32,
        path: String,
    ) -> Result<FileEntry, ServerStatFileError> {
        let storage = StorageIdentifier::Primary(server_id);
        stat_file(self, storage, path).await
    }

    async fn server_rename_file(
        &self,
        server_id: u32,
        from: String,
        to: String,
    ) -> Result<(), ServerRenameFileError> {
        let storage = StorageIdentifier::Primary(server_id);
        rename_file(self, storage, from, to).await
    }

    async fn server_copy_file(
        &self,
        server_id: u32,
        from: String,
        to: String,
    ) -> Result<(), ServerCopyFileError> {
        let storage = StorageIdentifier::Primary(server_id);
        copy_file(self, storage, from, to).await
    }

    async fn server_download_file(
        &self,
        server_id: u32,
        path: String,
        limit: Option<u64>,
    ) -> Result<FileResponse, ServerDownloadFileError> {
        let storage = StorageIdentifier::Primary(server_id);
        download_file(self, storage, path, limit).await
    }

    async fn server_download_file_user(
        &self,
        server_id: u32,
        path: String,
        limit: Option<u64>,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<FileResponse, ServerDownloadFileUserError> {
        verify.verify_server_file_download::<ServerDownloadFileUserError>(
            self.root.user_key.as_ref(),
            timestamp,
            &self.key.id,
            server_id,
            &path,
        )?;
        let storage = StorageIdentifier::Primary(server_id);
        download_file(self, storage, path, limit).await
    }

    async fn server_upload_file(
        &self,
        server_id: u32,
        path: String,
        length: Option<TypedHeader<ContentLength>>,
        data: DataStream,
    ) -> Result<(), ServerUploadFileError> {
        let storage = StorageIdentifier::Primary(server_id);
        write_file(self, storage, path, length, data).await
    }

    async fn server_upload_file_user(
        &self,
        server_id: u32,
        path: String,
        timestamp: i64,
        verify: UserSignature,
        length: Option<TypedHeader<ContentLength>>,
        data: DataStream,
    ) -> Result<(), ServerUploadFileUserError> {
        verify.verify_server_file_upload::<ServerUploadFileUserError>(
            self.root.user_key.as_ref(),
            timestamp,
            &self.key.id,
            server_id,
            &path,
        )?;
        let storage = StorageIdentifier::Primary(server_id);
        write_file(self, storage, path, length, data).await
    }

    async fn server_write_file_by_url(
        &self,
        server_id: u32,
        path: String,
        url: Url,
    ) -> Result<(), ServerWriteFileByUrlError> {
        let storage = StorageIdentifier::Primary(server_id);
        write_file_by_url(self, storage, path, url).await
    }

    async fn server_write_file_by_template(
        &self,
        server_id: u32,
        path: String,
        template: ServerFileWrite,
    ) -> Result<(), ServerWriteFileByTemplateError> {
        let storage = StorageIdentifier::Primary(server_id);
        write_file_by_template(self, storage, path, template).await
    }

    async fn server_delete_file(
        &self,
        server_id: u32,
        path: String,
        recursive: bool,
    ) -> Result<(), ServerDeleteFileError> {
        let storage = StorageIdentifier::Primary(server_id);
        delete_file(self, storage, path, recursive).await
    }

    async fn server_create_directory(
        &self,
        server_id: u32,
        path: String,
        recursive: bool,
    ) -> Result<(), ServerCreateDirectoryError> {
        let storage = StorageIdentifier::Primary(server_id);
        mkdir_file(self, storage, path, recursive).await
    }

    async fn server_symlink(
        &self,
        server_id: u32,
        path: String,
        content: String,
    ) -> Result<(), ServerSymlinkError> {
        let storage = StorageIdentifier::Primary(server_id);
        symlink_file(self, storage, path, content).await
    }

    async fn server_unpack_archive(
        &self,
        server_id: u32,
        path: String,
        dest: String,
    ) -> Result<(), ServerUnpackArchiveError> {
        let storage = StorageIdentifier::Primary(server_id);
        unpack_archive(self, storage, path, dest).await
    }

    async fn server_create_archive(
        &self,
        server_id: u32,
        src: String,
        path: String,
        compression: String,
    ) -> Result<(), ServerCreateArchiveError> {
        let storage = StorageIdentifier::Primary(server_id);
        create_archive(self, storage, path, src, compression).await
    }
}
