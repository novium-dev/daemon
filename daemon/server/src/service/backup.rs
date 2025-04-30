use crate::storage::StorageIdentifier;
use crate::{Daemon, storage};
use async_trait::async_trait;
use daemon_common::FileResponse;
use daemon_common::payload::backup::BackupResponseEntry;
use daemon_common::service::backup::BackupService;
use daemon_common::service::backup::error::{
    CreateBackupError, CreateBackupOrderError, DeleteBackupError, DownloadBackupError,
    DownloadBackupUserError, DownloadRealtimeBackupError, DownloadRealtimeBackupUserError,
    ListBackupsError, RestoreBackupError, SetBackupNameError,
};
use daemon_common::user::UserSignature;

#[async_trait]
impl BackupService for Daemon {
    async fn create_backup(
        &self,
        server_id: u32,
        backup_storage: String,
    ) -> Result<i64, CreateBackupError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::create_backup(self, storage, &backup_storage).await
    }

    async fn create_backup_order(&self, server_id: u32) -> Result<i64, CreateBackupOrderError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::create_backup_order(self, storage).await
    }

    async fn download_backup(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
    ) -> Result<FileResponse, DownloadBackupError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::backup_to_stream(self, storage, backup_id, &backup_storage).await
    }

    async fn download_backup_user(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<FileResponse, DownloadBackupUserError> {
        verify.verify_backup_download::<DownloadBackupUserError>(
            self.config.user_key,
            timestamp,
            &self.config.identifier,
            server_id,
            &backup_storage,
            backup_id,
        )?;
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::backup_to_stream(self, storage, backup_id, &backup_storage).await
    }

    async fn download_realtime_backup(
        &self,
        server_id: u32,
        compression: String,
    ) -> Result<FileResponse, DownloadRealtimeBackupError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::stream_realtime(self, storage, compression).await
    }

    async fn download_realtime_backup_user(
        &self,
        server_id: u32,
        timestamp: i64,
        verify: UserSignature,
        compression: String,
    ) -> Result<FileResponse, DownloadRealtimeBackupUserError> {
        verify.verify_backup_realtime_download::<DownloadRealtimeBackupUserError>(
            self.config.user_key,
            timestamp,
            &self.config.identifier,
            server_id,
            &compression,
        )?;
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::stream_realtime(self, storage, compression).await
    }

    async fn delete_backup(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
    ) -> Result<(), DeleteBackupError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::delete_backup(self, storage, backup_id, &backup_storage).await
    }

    async fn list_backups(
        &self,
        server_id: u32,
    ) -> Result<Vec<BackupResponseEntry>, ListBackupsError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::list_backups(self, storage).await
    }

    async fn restore_backup(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
    ) -> Result<(), RestoreBackupError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::restore_backup(self, storage, backup_id, &backup_storage).await
    }

    async fn set_backup_name(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
        name: Option<String>,
    ) -> Result<(), SetBackupNameError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::set_backup_name(self, storage, backup_id, &backup_storage, name).await
    }
}
