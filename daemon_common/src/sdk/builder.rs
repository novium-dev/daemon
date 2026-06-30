// SPDX-License-Identifier: AGPL-3.0-only
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
        // Internal base URL used for requests.
        Url,
        // Public base URL used for signed URLs, or `None` to reuse the internal URL.
        Option<Url>,
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
    /// Sets the base URL used for requests. Unless overridden with
    /// [`DaemonSDKBuilder::with_public_url`], signed URLs reuse this URL as well.
    pub fn with_url(self, mut url: Url) -> DaemonSDKBuilder<()> {
        base_url(&mut url);
        DaemonSDKBuilder((self.0.0, self.0.1, self.0.2, url, None))
    }
}

impl DaemonSDKBuilder<()> {
    /// Overrides the public base URL used for signed URLs, leaving the request URL untouched.
    pub fn with_public_url(self, mut public_url: Url) -> Self {
        base_url(&mut public_url);
        DaemonSDKBuilder((self.0.0, self.0.1, self.0.2, self.0.3, Some(public_url)))
    }

    pub fn with_user_key(self, user_key: SigningKey) -> DaemonSDKBuilder<WithUserKey> {
        DaemonSDKBuilder((self.0.0, self.0.1, self.0.2, self.0.3, self.0.4, user_key))
    }

    pub fn build(self) -> DaemonSDK<()> {
        Self::build0(self.0.0, self.0.1, &self.0.2, self.0.3, self.0.4, ())
    }
}

impl<T: DaemonSDKExtra + DaemonSDKBuilderState + Send + Sync> DaemonSDKBuilder<T> {
    pub(crate) fn build0(
        identifier: Uuid,
        root_key: Either<Arc<DaemonRootKey>, Arc<RootCertStore>>,
        backend_key: &BackendKey,
        base_url: Url,
        public_url: Option<Url>,
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
                WebPkiServerVerifier::builder(cert_store)
                    .build()
                    .expect("Unable to build webpki server verifier"),
            )))
            .with_client_auth_cert(
                backend_key.tls_cert.cert.clone(),
                backend_key.privkey.clone_key(),
            )
            .expect("Unable to build client config");
        tls.enable_sni = true;
        tls.alpn_protocols = vec!["h2".into()];
        DaemonSDK {
            client: Client::builder()
                .no_hickory_dns()
                .zstd(true)
                .https_only(true)
                .http2_prior_knowledge()
                .tls_backend_rustls()
                .tls_certs_only([])
                .tls_version_min(reqwest::tls::Version::TLS_1_3)
                .tls_sni(true)
                .tls_backend_preconfigured(tls)
                .with_server_name_resolver(Arc::new(DaemonSDKServerNameResolver(identifier)))
                .build()
                .expect("Unable to build reqwest client"),
            identifier,
            base_url,
            public_url,
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
        Option<Url>,
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
            self.0.4,
            WithUserKey(self.0.5),
        )
    }
}
