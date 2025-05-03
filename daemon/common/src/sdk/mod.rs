use ed25519_dalek::SigningKey;

pub mod builder;

use crate::sdk::builder::WithUserKey;
use crate::service::backup::BackupServiceClientImpl;
use crate::service::database::DatabaseServiceClientImpl;
use crate::service::files::FilesServiceClientImpl;
use crate::service::network::NetworkServiceClientImpl;
use crate::service::resource::ResourceServiceClientImpl;
use crate::service::schedule::ScheduleServiceClientImpl;
use crate::service::server::ServerServiceClientImpl;
use crate::service::setup::SetupServiceClientImpl;
use crate::service::{
    DaemonServices, DaemonServicesClientImpl, DaemonServicesURL, DaemonServicesURLWithUserKey,
};
use crate::user::UserKeyProvider;
use daemon_framework::FrameworkClientImpl;
use reqwest::{Client, Url};

#[allow(private_bounds)]
pub trait DaemonSDKExtra: DaemonSdkSealed + Sized {}
pub(crate) trait DaemonSdkSealed {}
impl DaemonSDKExtra for () {}
impl DaemonSdkSealed for () {}

#[derive(Debug, Clone)]
pub struct DaemonSDK<T: DaemonSDKExtra + Send + Sync> {
    client: Client,
    identifier: String,
    base_url: Url,
    extra: T,
}

impl<T: DaemonSDKExtra + Send + Sync> FrameworkClientImpl for DaemonSDK<T> {
    fn client(&self) -> &Client {
        &self.client
    }

    fn base_url(&self) -> &Url {
        &self.base_url
    }
}

impl<T: DaemonSDKExtra + Send + Sync> DaemonServicesClientImpl for DaemonSDK<T> {}
impl<T: DaemonSDKExtra + Send + Sync> ServerServiceClientImpl for DaemonSDK<T> {}
impl<T: DaemonSDKExtra + Send + Sync> BackupServiceClientImpl for DaemonSDK<T> {}
impl<T: DaemonSDKExtra + Send + Sync> FilesServiceClientImpl for DaemonSDK<T> {}
impl<T: DaemonSDKExtra + Send + Sync> NetworkServiceClientImpl for DaemonSDK<T> {}
impl<T: DaemonSDKExtra + Send + Sync> ResourceServiceClientImpl for DaemonSDK<T> {}
impl<T: DaemonSDKExtra + Send + Sync> DatabaseServiceClientImpl for DaemonSDK<T> {}
impl<T: DaemonSDKExtra + Send + Sync> ScheduleServiceClientImpl for DaemonSDK<T> {}
impl<T: DaemonSDKExtra + Send + Sync> SetupServiceClientImpl for DaemonSDK<T> {}
impl<T: DaemonSDKExtra + Send + Sync> DaemonServices for DaemonSDK<T> {}
impl<T: DaemonSDKExtra + Send + Sync> DaemonServicesURL for DaemonSDK<T> {}
impl DaemonServicesURLWithUserKey for DaemonSDK<WithUserKey> {}

impl UserKeyProvider for DaemonSDK<WithUserKey> {
    fn node_id(&self) -> &str {
        &self.identifier
    }

    fn key(&self) -> &SigningKey {
        &self.extra.0
    }
}
