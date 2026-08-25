// SPDX-License-Identifier: AGPL-3.0-only
use core::time::Duration;
use ed25519_dalek::SigningKey;

pub mod builder;

use crate::auth::{Attestation, AuthError, Challenge};
use crate::key::BackendKey;
use crate::sdk::builder::{Transport, WithUserKey};
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
use reqwest::{Client, Url};
use rustls::RootCertStore;
use std::sync::Arc;
use uuid::Uuid;
use zelus::sdk::ZelusClientImpl;

pub trait DaemonSDKExtra: DaemonSdkSealed + Sized {}
pub(crate) trait DaemonSdkSealed {}
impl DaemonSDKExtra for () {}
impl DaemonSdkSealed for () {}

pub type DefaultDaemonSDK = DaemonSDK<()>;
pub type DaemonSDKWithUserKey = DaemonSDK<WithUserKey>;

#[derive(Debug, Clone)]
#[must_use]
pub struct DaemonSDK<T: DaemonSDKExtra + Send + Sync> {
    pub(crate) client: Client,
    pub(crate) identifier: Uuid,
    pub(crate) base_url: Url,
    pub(crate) cert_store: Arc<RootCertStore>,
    pub(crate) backend_key: Arc<BackendKey>,
    pub(crate) transport: Transport,
    pub(crate) token_validity: Duration,
    pub(crate) token_expires_at: i64,
    pub(crate) extra: T,
}

#[derive(Debug, thiserror::Error)]
pub enum AttestationError {
    #[error("unable to reach the node: {0}")]
    Request(#[from] reqwest::Error),
    #[error("node did not return an attestation")]
    Missing,
    #[error("attestation is not valid: {0}")]
    Invalid(#[from] AuthError),
}

impl<T: DaemonSDKExtra + Send + Sync> DaemonSDK<T> {
    /// Unix timestamp at which the embedded node token stops being accepted.
    #[must_use]
    pub const fn token_expires_at(&self) -> i64 {
        self.token_expires_at
    }

    /// Reissues the node token and rebuilds the underlying HTTP client.
    ///
    /// The token lives in the client's default headers, so it cannot be swapped
    /// out in place; call this before [`Self::token_expires_at`] is reached.
    pub fn renew(&mut self)
    where
        T: Clone + builder::DaemonSDKBuilderState,
    {
        let renewed = builder::DaemonSDKBuilder::<T>::build0(
            self.identifier,
            either::Either::Right(Arc::clone(&self.cert_store)),
            Arc::clone(&self.backend_key),
            self.base_url.clone(),
            self.transport,
            self.token_validity,
            self.extra.clone(),
        );
        self.client = renewed.client;
        self.token_expires_at = renewed.token_expires_at;
    }

    /// Asks the node to prove it holds the private key for `node-<uuid>`.
    ///
    /// For [`Transport::Proxied`] connections this is what establishes node
    /// identity, since the TLS certificate belongs to the proxy rather than to
    /// the node. Call it once after building the SDK, and again whenever the
    /// identity needs to be reconfirmed.
    pub async fn attest(&self) -> Result<(), AttestationError> {
        let challenge = Challenge::generate();

        let mut url = self
            .base_url
            .join(crate::DAEMON_ATTEST_PATH)
            .expect("attest endpoint url is valid");
        url.query_pairs_mut()
            .append_pair("challenge", &challenge.to_string());

        let response = self.client.get(url).send().await?.error_for_status()?;

        let attestation = response
            .headers()
            .get(crate::DAEMON_ATTESTATION_HEADER)
            .and_then(|value| value.to_str().ok())
            .ok_or(AttestationError::Missing)?
            .parse::<Attestation>()?;

        attestation.verify(&self.cert_store, self.identifier, challenge)?;
        Ok(())
    }
}

impl<T: DaemonSDKExtra + Send + Sync> ZelusClientImpl for DaemonSDK<T> {
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
