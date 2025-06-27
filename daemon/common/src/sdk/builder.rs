use crate::key::{BackendKey, DaemonKeyVerifier, DaemonRootKey};
use crate::sdk::{DaemonSDK, DaemonSDKExtra, DaemonSDKServerNameResolver, DaemonSdkSealed};
use crate::utils::base_url;
use ed25519_dalek::SigningKey;
use either::Either;
use reqwest::{Client, Url};
use rustls::client::WebPkiServerVerifier;
use rustls::{ClientConfig, RootCertStore};
use std::sync::Arc;
use uuid::Uuid;

#[expect(private_bounds, reason = "trait should be sealed")]
pub trait DaemonSDKBuilderState: DaemonSdkSealed {
    type Inner;
}
pub enum RequiresIdentifier {}
pub enum RequiresRootKey {}
pub enum RequiresBackendKey {}
pub enum RequiresBaseURL {}

#[derive(Clone)]
pub struct WithUserKey(pub(crate) SigningKey);

impl DaemonSDKExtra for WithUserKey {}
impl DaemonSdkSealed for WithUserKey {}

impl DaemonSDKBuilderState for RequiresIdentifier {
    type Inner = ();
}
impl DaemonSDKBuilderState for RequiresRootKey {
    type Inner = Uuid;
}
impl DaemonSDKBuilderState for RequiresBackendKey {
    type Inner = (Uuid, Either<Arc<DaemonRootKey>, Arc<RootCertStore>>);
}
impl DaemonSDKBuilderState for RequiresBaseURL {
    type Inner = (
        Uuid,
        Either<Arc<DaemonRootKey>, Arc<RootCertStore>>,
        Arc<BackendKey>,
    );
}
impl DaemonSDKBuilderState for () {
    type Inner = (
        Uuid,
        Either<Arc<DaemonRootKey>, Arc<RootCertStore>>,
        Arc<BackendKey>,
        Url,
    );
}
impl DaemonSdkSealed for RequiresIdentifier {}
impl DaemonSdkSealed for RequiresRootKey {}
impl DaemonSdkSealed for RequiresBackendKey {}
impl DaemonSdkSealed for RequiresBaseURL {}

#[must_use]
pub struct DaemonSDKBuilder<T: DaemonSDKBuilderState + Send + Sync>(pub(crate) T::Inner);

impl DaemonSDK<()> {
    pub const fn builder() -> DaemonSDKBuilder<RequiresIdentifier> {
        DaemonSDKBuilder(())
    }
}

impl DaemonSDKBuilder<RequiresIdentifier> {
    pub const fn with_identifier(self, identifier: Uuid) -> DaemonSDKBuilder<RequiresRootKey> {
        DaemonSDKBuilder(identifier)
    }
}

impl DaemonSDKBuilder<RequiresRootKey> {
    pub const fn with_root_key(
        self,
        root: Arc<DaemonRootKey>,
    ) -> DaemonSDKBuilder<RequiresBackendKey> {
        DaemonSDKBuilder((self.0, Either::Left(root)))
    }

    pub const fn with_custom_cert_store(
        self,
        der: Arc<RootCertStore>,
    ) -> DaemonSDKBuilder<RequiresBackendKey> {
        DaemonSDKBuilder((self.0, Either::Right(der)))
    }
}

impl DaemonSDKBuilder<RequiresBackendKey> {
    pub fn with_backend_key(self, backend: Arc<BackendKey>) -> DaemonSDKBuilder<RequiresBaseURL> {
        DaemonSDKBuilder((self.0.0, self.0.1, backend))
    }
}

impl DaemonSDKBuilder<RequiresBaseURL> {
    pub fn with_url(self, mut url: Url) -> DaemonSDKBuilder<()> {
        base_url(&mut url);
        DaemonSDKBuilder((self.0.0, self.0.1, self.0.2, url))
    }
}

impl DaemonSDKBuilder<()> {
    pub fn with_user_key(self, user_key: SigningKey) -> DaemonSDKBuilder<WithUserKey> {
        DaemonSDKBuilder((self.0.0, self.0.1, self.0.2, self.0.3, user_key))
    }

    pub fn build(self) -> DaemonSDK<()> {
        Self::build0(self.0.0, self.0.1, &self.0.2, self.0.3, ())
    }
}

impl<T: DaemonSDKExtra + DaemonSDKBuilderState + Send + Sync> DaemonSDKBuilder<T> {
    pub(crate) fn build0(
        identifier: Uuid,
        root_key: Either<Arc<DaemonRootKey>, Arc<RootCertStore>>,
        backend_key: &BackendKey,
        base_url: Url,
        extra: T,
    ) -> DaemonSDK<T> {
        let cert_store = match root_key {
            Either::Left(key) => Arc::clone(&key.cert_store),
            Either::Right(cert_store) => cert_store,
        };
        let mut tls = ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(DaemonKeyVerifier(
                identifier,
                WebPkiServerVerifier::builder(cert_store).build().unwrap(),
            )))
            .with_client_auth_cert(
                backend_key.tls_cert.cert.clone(),
                backend_key.privkey.clone_key(),
            )
            .unwrap();
        tls.enable_sni = true;
        tls.alpn_protocols = vec!["h2".into()];
        DaemonSDK {
            client: Client::builder()
                .tls_built_in_root_certs(false)
                .https_only(true)
                .use_preconfigured_tls(tls)
                .with_server_name_resolver(Arc::new(DaemonSDKServerNameResolver(identifier)))
                .build()
                .unwrap(),
            identifier,
            base_url,
            extra,
        }
    }
}

impl DaemonSDKBuilderState for WithUserKey {
    type Inner = (
        Uuid,
        Either<Arc<DaemonRootKey>, Arc<RootCertStore>>,
        Arc<BackendKey>,
        Url,
        SigningKey,
    );
}

impl DaemonSDKBuilder<WithUserKey> {
    pub fn build(self) -> DaemonSDK<WithUserKey> {
        Self::build0(
            self.0.0,
            self.0.1,
            &self.0.2,
            self.0.3,
            WithUserKey(self.0.4),
        )
    }
}
