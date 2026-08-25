// SPDX-License-Identifier: AGPL-3.0-only
use crate::auth::NodeToken;
use crate::key::{BackendKey, DaemonKeyVerifier, DaemonRootKey};
use crate::sdk::{DaemonSDK, DaemonSDKExtra, DaemonSdkSealed};
use crate::utils::base_url;
use core::time::Duration;
use ed25519_dalek::SigningKey;
use either::Either;
use reqwest::header::{HeaderMap, HeaderValue};
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

/// How the backend reaches a node, which decides how the TLS peer is validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// The backend opens a connection straight to the node.
    ///
    /// The node serves its own `node-<uuid>` certificate, so the certificate is
    /// validated against the daemon root CA and pinned to the node identifier.
    /// SNI is switched off because the URL host is an address rather than the
    /// name inside the certificate; the daemon falls back to the node
    /// certificate when no server name is offered.
    Direct,
    /// The connection is terminated by a reverse proxy holding its own publicly
    /// trusted certificate.
    ///
    /// TLS is then validated normally against the platform trust store, and the
    /// node identity is established at the application layer instead, by
    /// [`DaemonSDK::attest`](crate::sdk::DaemonSDK::attest).
    Proxied,
}

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
pub struct DaemonSDKBuilder<T: DaemonSDKBuilderState + Send + Sync> {
    pub(crate) inner: T::Inner,
    pub(crate) transport: Transport,
    pub(crate) token_validity: Duration,
}

impl<T: DaemonSDKBuilderState + Send + Sync> DaemonSDKBuilder<T> {
    const fn advance<N: DaemonSDKBuilderState + Send + Sync>(
        self,
        inner: N::Inner,
    ) -> DaemonSDKBuilder<N> {
        DaemonSDKBuilder {
            inner,
            transport: self.transport,
            token_validity: self.token_validity,
        }
    }

    /// Selects how the node is reached. Defaults to [`Transport::Direct`].
    pub const fn with_transport(mut self, transport: Transport) -> Self {
        self.transport = transport;
        self
    }

    /// Overrides how long the issued node token stays valid.
    ///
    /// The token is embedded in the client's default headers, so the SDK has to
    /// be rebuilt (or [`DaemonSDK::renew`](crate::sdk::DaemonSDK::renew) called)
    /// before it expires.
    pub const fn with_token_validity(mut self, validity: Duration) -> Self {
        self.token_validity = validity;
        self
    }
}

impl DaemonSDK<()> {
    pub const fn builder() -> DaemonSDKBuilder<RequiresIdentifier> {
        DaemonSDKBuilder {
            inner: (),
            transport: Transport::Direct,
            token_validity: crate::DAEMON_NODE_TOKEN_VALIDITY,
        }
    }
}

impl DaemonSDKBuilder<RequiresIdentifier> {
    pub const fn with_identifier(self, identifier: Uuid) -> DaemonSDKBuilder<RequiresRootKey> {
        self.advance::<RequiresRootKey>(identifier)
    }
}

impl DaemonSDKBuilder<RequiresRootKey> {
    pub fn with_root_key(self, root: Arc<DaemonRootKey>) -> DaemonSDKBuilder<RequiresBackendKey> {
        let identifier = self.inner;
        self.advance::<RequiresBackendKey>((identifier, Either::Left(root)))
    }

    pub fn with_custom_cert_store(
        self,
        der: Arc<RootCertStore>,
    ) -> DaemonSDKBuilder<RequiresBackendKey> {
        let identifier = self.inner;
        self.advance::<RequiresBackendKey>((identifier, Either::Right(der)))
    }
}

impl DaemonSDKBuilder<RequiresBackendKey> {
    pub fn with_backend_key(self, backend: Arc<BackendKey>) -> DaemonSDKBuilder<RequiresBaseURL> {
        let (identifier, root) = self.inner;
        self.advance::<RequiresBaseURL>((identifier, root, backend))
    }
}

impl DaemonSDKBuilder<RequiresBaseURL> {
    pub fn with_url(self, mut url: Url) -> DaemonSDKBuilder<()> {
        base_url(&mut url);
        let (identifier, root, backend) = self.inner;
        self.advance::<()>((identifier, root, backend, url))
    }
}

impl DaemonSDKBuilder<()> {
    pub fn with_user_key(self, user_key: SigningKey) -> DaemonSDKBuilder<WithUserKey> {
        let (identifier, root, backend, url) = self.inner;
        self.advance::<WithUserKey>((identifier, root, backend, url, user_key))
    }

    pub fn build(self) -> DaemonSDK<()> {
        let transport = self.transport;
        let token_validity = self.token_validity;
        let (identifier, root, backend, url) = self.inner;
        DaemonSDKBuilder::<()>::build0(
            identifier,
            root,
            backend,
            url,
            transport,
            token_validity,
            (),
        )
    }
}

/// Builds the rustls configuration used for a [`Transport::Direct`] connection.
///
/// This pins the peer to `node-<identifier>` signed by the daemon root CA, which
/// is the same guarantee the mutual TLS setup provided for the server direction.
fn direct_tls(identifier: Uuid, cert_store: Arc<RootCertStore>) -> ClientConfig {
    let mut tls = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(DaemonKeyVerifier(
            identifier,
            WebPkiServerVerifier::builder(cert_store)
                .build()
                .expect("Unable to build webpki server verifier"),
        )))
        .with_no_client_auth();
    // The node certificate name is supplied by the verifier, not by SNI, so the
    // handshake must not advertise the (unrelated) URL host as a server name.
    tls.enable_sni = false;
    tls.alpn_protocols = vec!["h2".into()];
    tls
}

impl<T: DaemonSDKExtra + DaemonSDKBuilderState + Send + Sync> DaemonSDKBuilder<T> {
    pub(crate) fn build0(
        identifier: Uuid,
        root_key: Either<Arc<DaemonRootKey>, Arc<RootCertStore>>,
        backend_key: Arc<BackendKey>,
        base_url: Url,
        transport: Transport,
        token_validity: Duration,
        extra: T,
    ) -> DaemonSDK<T> {
        let cert_store = match root_key {
            Either::Left(key) => Arc::clone(&key.cert_store),
            Either::Right(cert_store) => cert_store,
        };

        let token = NodeToken::issue(&backend_key.tls_cert, identifier, token_validity)
            .expect("Unable to issue node token");
        let expires_at = token.expires_at();

        let mut headers = HeaderMap::new();
        let mut value = HeaderValue::from_str(&token.to_string())
            .expect("Node token is not a valid header value");
        value.set_sensitive(true);
        headers.insert(crate::DAEMON_AUTH_HEADER, value);

        let client = Client::builder()
            .no_hickory_dns()
            .zstd(true)
            .https_only(true)
            .http2_prior_knowledge()
            .default_headers(headers)
            .tls_backend_rustls()
            .tls_version_min(reqwest::tls::Version::TLS_1_3);

        let client = match transport {
            // A proxy presents its own publicly trusted certificate, so leave
            // verification and SNI at the stock reqwest defaults.
            Transport::Proxied => client,
            Transport::Direct => client
                .tls_certs_only([])
                .tls_backend_preconfigured(direct_tls(identifier, Arc::clone(&cert_store))),
        };

        DaemonSDK {
            client: client.build().expect("Unable to build reqwest client"),
            identifier,
            base_url,
            cert_store,
            backend_key,
            transport,
            token_validity,
            token_expires_at: expires_at,
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
        let transport = self.transport;
        let token_validity = self.token_validity;
        let (identifier, root, backend, url, user_key) = self.inner;
        Self::build0(
            identifier,
            root,
            backend,
            url,
            transport,
            token_validity,
            WithUserKey(user_key),
        )
    }
}
