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
use core::error::Error;
use ed25519_dalek::SigningKey;
use http::Uri;

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
use daemon_framework::sdk::FrameworkClientImpl;
use reqwest::{Client, ResolveServerName, Url};
use rustls::pki_types::{DnsName, ServerName};
use uuid::Uuid;

pub trait DaemonSDKExtra: DaemonSdkSealed + Sized {}
pub(crate) trait DaemonSdkSealed {}
impl DaemonSDKExtra for () {}
impl DaemonSdkSealed for () {}

pub type DefaultDaemonSDK = DaemonSDK<()>;
pub type DaemonSDKWithUserKey = DaemonSDK<WithUserKey>;

#[derive(Debug, Clone)]
#[must_use]
pub struct DaemonSDK<T: DaemonSDKExtra + Send + Sync> {
    client: Client,
    identifier: Uuid,
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
impl<T: DaemonSDKExtra + Send + Sync> DaemonServices for DaemonSDK<T> {
    fn id(&self) -> Uuid {
        self.identifier
    }
}
impl<T: DaemonSDKExtra + Send + Sync> DaemonServicesURL for DaemonSDK<T> {}
impl DaemonServicesURLWithUserKey for DaemonSDK<WithUserKey> {}

impl UserKeyProvider for DaemonSDK<WithUserKey> {
    fn node_id(&self) -> &Uuid {
        &self.identifier
    }

    fn key(&self) -> &SigningKey {
        &self.extra.0
    }
}

pub(crate) struct DaemonSDKServerNameResolver(pub Uuid);

impl ResolveServerName for DaemonSDKServerNameResolver {
    fn resolve(&self, _uri: &Uri) -> Result<ServerName<'static>, Box<dyn Error + Sync + Send>> {
        Ok(ServerName::DnsName(DnsName::try_from(format!(
            "node-{}",
            self.0
        ))?))
    }
}
