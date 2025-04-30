use chrono::Utc;
use daemon_common::user::UserSignature;
use crate::*;
use crate::builder::user::WithUserKey;

impl DaemonSDK<WithUserKey> {

    pub fn download_backup_user_url_signed(&self, server_id: u32, backup_storage: String, backup_id: i64) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature = UserSignature::new_backup_download(&self.extra.0, timestamp, &self.identifier, server_id, &backup_storage, backup_id);
        self.download_backup_user_url(server_id, backup_storage, backup_id, timestamp, signature)
    }
    
    pub fn download_realtime_backup_user_url_signed(&self, server_id: u32, compression: String) -> Url {
        let timestamp = Utc::now().timestamp();
        let signature = UserSignature::new_backup_realtime_download(&self.extra.0, timestamp, &self.identifier, server_id, &compression);
        self.download_realtime_backup_user_url(server_id, timestamp, signature, compression)
    }

}
