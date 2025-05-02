use crate::storage::StorageIdentifier;
use crate::{Daemon, storage};
use async_trait::async_trait;
use daemon_common::FileResponse;
use daemon_common::payload::backup::BackupResponseEntry;
use daemon_common::service::backup::BackupService;
use daemon_common::service::backup::error::{
    ServerCreateBackupError, ServerCreateBackupOrderError, ServerDeleteBackupError, ServerDownloadBackupError,
    ServerDownloadBackupUserError, ServerDownloadRealtimeError, ServerDownloadRealtimeUserError,
    ServerListBackupsError, ServerRestoreBackupError, ServerSetBackupNameError,
};
use daemon_common::user::UserSignature;

#[async_trait]
impl BackupService for Daemon {
    async fn server_list_backups(
        &self,
        server_id: u32,
    ) -> Result<Vec<BackupResponseEntry>, ServerListBackupsError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::list_backups(self, storage).await
    }

    async fn server_create_backup_order(&self, server_id: u32) -> Result<i64, ServerCreateBackupOrderError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::create_backup_order(self, storage).await
    }

    async fn server_create_backup(
        &self,
        server_id: u32,
        backup_storage: String,
    ) -> Result<i64, ServerCreateBackupError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::create_backup(self, storage, &backup_storage).await
    }

    async fn server_download_backup(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
    ) -> Result<FileResponse, ServerDownloadBackupError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::backup_to_stream(self, storage, backup_id, &backup_storage).await
    }

    async fn server_download_backup_user(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
        timestamp: i64,
        verify: UserSignature,
    ) -> Result<FileResponse, ServerDownloadBackupUserError> {
        verify.verify_server_backup_download::<ServerDownloadBackupUserError>(
            &self.config.user_key,
            timestamp,
            &self.config.identifier,
            server_id,
            &backup_storage,
            backup_id,
        )?;
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::backup_to_stream(self, storage, backup_id, &backup_storage).await
    }

    async fn server_download_realtime(
        &self,
        server_id: u32,
        compression: String,
    ) -> Result<FileResponse, ServerDownloadRealtimeError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::stream_realtime(self, storage, compression).await
    }

    async fn server_download_realtime_user(
        &self,
        server_id: u32,
        timestamp: i64,
        verify: UserSignature,
        compression: String,
    ) -> Result<FileResponse, ServerDownloadRealtimeUserError> {
        verify.verify_server_realtime_download::<ServerDownloadRealtimeUserError>(
            &self.config.user_key,
            timestamp,
            &self.config.identifier,
            server_id,
            &compression,
        )?;
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::stream_realtime(self, storage, compression).await
    }

    async fn server_set_backup_name(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
        name: Option<String>,
    ) -> Result<(), ServerSetBackupNameError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::set_backup_name(self, storage, backup_id, &backup_storage, name).await
    }

    async fn server_delete_backup(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
    ) -> Result<(), ServerDeleteBackupError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::delete_backup(self, storage, backup_id, &backup_storage).await
    }

    async fn server_restore_backup(
        &self,
        server_id: u32,
        backup_storage: String,
        backup_id: i64,
    ) -> Result<(), ServerRestoreBackupError> {
        let storage = StorageIdentifier::Primary(server_id);
        storage::backup::restore_backup(self, storage, backup_id, &backup_storage).await
    }
}
