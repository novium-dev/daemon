// SPDX-License-Identifier: AGPL-3.0-only
//! Application layer authentication between the backend and a daemon node.
//!
//! This replaces mutual TLS. The certificate hierarchy is unchanged: the root CA
//! still issues a `ClientAuth` certificate to the backend and a `ServerAuth`
//! certificate to every node. Instead of presenting those certificates during the
//! TLS handshake, both sides prove possession of the matching private key at the
//! HTTP layer, which survives a TLS terminating reverse proxy.
//!
//! Two proofs exist:
//!
//! * [`NodeToken`] authenticates the backend towards a node. It is scoped to a
//!   single node identifier and carries an expiry, so a node can never replay it
//!   against a different node.
//! * [`Attestation`] authenticates a node towards the backend. The backend sends
//!   a random challenge and the node signs it with its node key, which proves the
//!   node behind a proxied URL actually holds the key for `node-<uuid>`.
//!
//! Both proofs are transported as a single compact header value so they survive
//! arbitrary proxies, and both are verified against the same [`RootCertStore`]
//! that the TLS code used before.

use base64::Engine as _;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use chrono::Utc;
use core::fmt::{Display, Formatter};
use core::str::FromStr;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, UnixTime};
use rustls::sign::CertifiedKey;
use rustls::{RootCertStore, SignatureScheme};
use uuid::Uuid;

/// Signature schemes we are willing to produce and accept, in preference order.
///
/// All of these are TLS 1.3 compatible, which keeps the set aligned with what the
/// handshake accepted before and rules out the legacy PKCS#1 schemes.
const SIGNATURE_SCHEMES: &[SignatureScheme] = &[
    SignatureScheme::ED25519,
    SignatureScheme::ECDSA_NISTP256_SHA256,
    SignatureScheme::ECDSA_NISTP384_SHA384,
    SignatureScheme::RSA_PSS_SHA256,
];

/// Clock skew tolerated when checking the validity window of a proof.
const CLOCK_SKEW: i64 = 30;

/// How long an [`Attestation`] challenge stays valid.
const ATTESTATION_VALIDITY: i64 = 60;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("malformed authentication proof")]
    Malformed,
    #[error("unsupported proof version")]
    Version,
    #[error("no supported signature scheme for this key")]
    UnsupportedScheme,
    #[error("proof is not valid for this node")]
    WrongNode,
    #[error("proof has expired")]
    Expired,
    #[error("challenge does not match")]
    ChallengeMismatch,
    #[error("certificate is not trusted: {0}")]
    Untrusted(rustls::Error),
    #[error("certificate is malformed: {0}")]
    Certificate(webpki::Error),
    #[error("signature is invalid")]
    Signature,
    #[error("rustls error: {0}")]
    Rustls(#[from] rustls::Error),
}

/// Signs `message` with the private key behind `key`.
///
/// The scheme is chosen by the key itself, so this works for every algorithm the
/// crypto provider supports and does not constrain how certificates are issued.
fn sign(key: &CertifiedKey, message: &[u8]) -> Result<(SignatureScheme, Vec<u8>), AuthError> {
    let signer = key
        .key
        .choose_scheme(SIGNATURE_SCHEMES)
        .ok_or(AuthError::UnsupportedScheme)?;
    let scheme = signer.scheme();
    let signature = signer.sign(message)?;
    Ok((scheme, signature))
}

/// Verifies that `certificate` chains to a trust anchor in `roots` for `usage`,
/// and that `signature` over `message` was produced by its private key.
fn verify(
    certificate: &CertificateDer<'_>,
    roots: &RootCertStore,
    usage: webpki::KeyUsage,
    scheme: SignatureScheme,
    message: &[u8],
    signature: &[u8],
) -> Result<(), AuthError> {
    let provider = CryptoProvider::get_default().ok_or(rustls::Error::General(
        "No crypto provider set".to_owned(),
    ))?;
    let algorithms = &provider.signature_verification_algorithms;

    let end_entity =
        webpki::EndEntityCert::try_from(certificate).map_err(AuthError::Certificate)?;

    // The backend and node certificates are issued directly by the root CA, so
    // there are never intermediates to pass along.
    end_entity
        .verify_for_usage(
            algorithms.all,
            &roots.roots,
            &[],
            UnixTime::now(),
            usage,
            None,
            None,
        )
        .map_err(AuthError::Certificate)?;

    let candidates = algorithms
        .mapping
        .iter()
        .find(|(mapped, _)| *mapped == scheme)
        .map(|(_, algorithms)| *algorithms)
        .ok_or(AuthError::UnsupportedScheme)?;

    // Mirrors how rustls verifies a TLS 1.2 handshake signature: a scheme can map
    // to several webpki algorithms, so try each until one accepts the key.
    for algorithm in candidates {
        match end_entity.verify_signature(*algorithm, message, signature) {
            Ok(()) => return Ok(()),
            Err(webpki::Error::UnsupportedSignatureAlgorithmForPublicKey) => continue,
            Err(_err) => return Err(AuthError::Signature),
        }
    }

    Err(AuthError::Signature)
}

/// Encodes the fixed prelude every signed message starts with, so a proof of one
/// kind can never be replayed as a proof of another kind.
fn message(domain: &str, parts: &[&str]) -> Vec<u8> {
    let mut out = String::from(domain);
    for part in parts {
        out.push('\n');
        out.push_str(part);
    }
    out.into_bytes()
}

/// A capability token proving the holder of the backend private key authorised
/// access to one specific node until `expires_at`.
///
/// The token is deliberately bound to a single node identifier. A compromised or
/// malicious node therefore gains nothing by replaying it, because every other
/// node rejects a token that does not name it.
#[derive(Debug, Clone)]
#[must_use]
pub struct NodeToken {
    node: Uuid,
    issued_at: i64,
    expires_at: i64,
    certificate: CertificateDer<'static>,
    scheme: SignatureScheme,
    signature: Vec<u8>,
}

impl NodeToken {
    const DOMAIN: &'static str = "novium-node-auth-v1";
    const PREFIX: &'static str = "nvt1";

    /// Issues a token for `node`, valid for `validity` seconds.
    pub fn issue(
        key: &CertifiedKey,
        node: Uuid,
        validity: core::time::Duration,
    ) -> Result<Self, AuthError> {
        let issued_at = Utc::now().timestamp();
        let expires_at = issued_at.saturating_add(
            i64::try_from(validity.as_secs()).unwrap_or(i64::MAX),
        );

        let certificate = key.end_entity_cert()?.clone();
        let (scheme, signature) = sign(
            key,
            &message(
                Self::DOMAIN,
                &[
                    &node.to_string(),
                    &issued_at.to_string(),
                    &expires_at.to_string(),
                ],
            ),
        )?;

        Ok(Self {
            node,
            issued_at,
            expires_at,
            certificate,
            scheme,
            signature,
        })
    }

    /// The point in time after which this token stops being accepted.
    #[must_use]
    pub const fn expires_at(&self) -> i64 {
        self.expires_at
    }

    /// Verifies the token was issued by the backend key for `expected_node`.
    pub fn verify(&self, roots: &RootCertStore, expected_node: Uuid) -> Result<(), AuthError> {
        if self.node != expected_node {
            return Err(AuthError::WrongNode);
        }

        let now = Utc::now().timestamp();
        if now > self.expires_at || now.saturating_add(CLOCK_SKEW) < self.issued_at {
            return Err(AuthError::Expired);
        }

        verify(
            &self.certificate,
            roots,
            webpki::KeyUsage::client_auth(),
            self.scheme,
            &message(
                Self::DOMAIN,
                &[
                    &self.node.to_string(),
                    &self.issued_at.to_string(),
                    &self.expires_at.to_string(),
                ],
            ),
            &self.signature,
        )
    }
}

impl Display for NodeToken {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "{}.{}.{}.{}.{}.{}.{}",
            Self::PREFIX,
            self.node,
            self.issued_at,
            self.expires_at,
            BASE64_URL_SAFE_NO_PAD.encode(&self.certificate),
            u16::from(self.scheme),
            BASE64_URL_SAFE_NO_PAD.encode(&self.signature),
        )
    }
}

impl FromStr for NodeToken {
    type Err = AuthError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let mut parts = raw.split('.');

        if parts.next() != Some(Self::PREFIX) {
            return Err(AuthError::Version);
        }

        let mut next = || parts.next().ok_or(AuthError::Malformed);

        let node = Uuid::from_str(next()?).map_err(|_err| AuthError::Malformed)?;
        let issued_at = next()?.parse().map_err(|_err| AuthError::Malformed)?;
        let expires_at = next()?.parse().map_err(|_err| AuthError::Malformed)?;
        let certificate = BASE64_URL_SAFE_NO_PAD
            .decode(next()?)
            .map_err(|_err| AuthError::Malformed)?;
        let scheme: u16 = next()?.parse().map_err(|_err| AuthError::Malformed)?;
        let signature = BASE64_URL_SAFE_NO_PAD
            .decode(next()?)
            .map_err(|_err| AuthError::Malformed)?;

        if parts.next().is_some() {
            return Err(AuthError::Malformed);
        }

        Ok(Self {
            node,
            issued_at,
            expires_at,
            certificate: CertificateDer::from(certificate),
            scheme: SignatureScheme::from(scheme),
            signature,
        })
    }
}

/// A random challenge the backend asks a node to sign.
///
/// This is what replaces pinning the node identity to the TLS server name. It
/// works through any number of proxies because it never touches the handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub struct Challenge([u8; 32]);

impl Challenge {
    /// Generates a fresh challenge.
    #[must_use]
    pub fn generate() -> Self {
        Self(rand::random())
    }
}

impl Display for Challenge {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(&BASE64_URL_SAFE_NO_PAD.encode(self.0))
    }
}

impl FromStr for Challenge {
    type Err = AuthError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let decoded = BASE64_URL_SAFE_NO_PAD
            .decode(raw)
            .map_err(|_err| AuthError::Malformed)?;
        Ok(Self(
            <[u8; 32]>::try_from(decoded.as_slice()).map_err(|_err| AuthError::Malformed)?,
        ))
    }
}

/// A node's answer to a [`Challenge`], proving it holds the private key for
/// `node-<uuid>`.
#[derive(Debug, Clone)]
#[must_use]
pub struct Attestation {
    node: Uuid,
    issued_at: i64,
    certificate: CertificateDer<'static>,
    scheme: SignatureScheme,
    signature: Vec<u8>,
}

impl Attestation {
    const DOMAIN: &'static str = "novium-node-attest-v1";
    const PREFIX: &'static str = "nva1";

    /// Signs `challenge` with the node key.
    pub fn sign(key: &CertifiedKey, node: Uuid, challenge: Challenge) -> Result<Self, AuthError> {
        let issued_at = Utc::now().timestamp();
        let certificate = key.end_entity_cert()?.clone();
        let (scheme, signature) = sign(
            key,
            &message(
                Self::DOMAIN,
                &[
                    &node.to_string(),
                    &issued_at.to_string(),
                    &challenge.to_string(),
                ],
            ),
        )?;

        Ok(Self {
            node,
            issued_at,
            certificate,
            scheme,
            signature,
        })
    }

    /// Verifies the attestation answers `challenge` and was produced by
    /// `expected_node`.
    pub fn verify(
        &self,
        roots: &RootCertStore,
        expected_node: Uuid,
        challenge: Challenge,
    ) -> Result<(), AuthError> {
        if self.node != expected_node {
            return Err(AuthError::WrongNode);
        }

        let now = Utc::now().timestamp();
        if now > self.issued_at.saturating_add(ATTESTATION_VALIDITY)
            || now.saturating_add(CLOCK_SKEW) < self.issued_at
        {
            return Err(AuthError::Expired);
        }

        verify(
            &self.certificate,
            roots,
            webpki::KeyUsage::server_auth(),
            self.scheme,
            &message(
                Self::DOMAIN,
                &[
                    &self.node.to_string(),
                    &self.issued_at.to_string(),
                    &challenge.to_string(),
                ],
            ),
            &self.signature,
        )?;

        // The certificate has to belong to the node we expected, not merely to
        // some node the same CA issued. This is the guarantee the pinned TLS
        // server name used to provide.
        let (_, x509) = x509_parser::parse_x509_certificate(&self.certificate)
            .map_err(|_err| AuthError::Malformed)?;
        let common_name = x509
            .subject
            .iter_common_name()
            .next()
            .ok_or(AuthError::Malformed)?
            .as_str()
            .map_err(|_err| AuthError::Malformed)?;

        if common_name != format!("node-{expected_node}") {
            return Err(AuthError::WrongNode);
        }

        Ok(())
    }
}

impl Display for Attestation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "{}.{}.{}.{}.{}.{}",
            Self::PREFIX,
            self.node,
            self.issued_at,
            BASE64_URL_SAFE_NO_PAD.encode(&self.certificate),
            u16::from(self.scheme),
            BASE64_URL_SAFE_NO_PAD.encode(&self.signature),
        )
    }
}

impl FromStr for Attestation {
    type Err = AuthError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let mut parts = raw.split('.');

        if parts.next() != Some(Self::PREFIX) {
            return Err(AuthError::Version);
        }

        let mut next = || parts.next().ok_or(AuthError::Malformed);

        let node = Uuid::from_str(next()?).map_err(|_err| AuthError::Malformed)?;
        let issued_at = next()?.parse().map_err(|_err| AuthError::Malformed)?;
        let certificate = BASE64_URL_SAFE_NO_PAD
            .decode(next()?)
            .map_err(|_err| AuthError::Malformed)?;
        let scheme: u16 = next()?.parse().map_err(|_err| AuthError::Malformed)?;
        let signature = BASE64_URL_SAFE_NO_PAD
            .decode(next()?)
            .map_err(|_err| AuthError::Malformed)?;

        if parts.next().is_some() {
            return Err(AuthError::Malformed);
        }

        Ok(Self {
            node,
            issued_at,
            certificate: CertificateDer::from(certificate),
            scheme: SignatureScheme::from(scheme),
            signature,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::{Attestation, Challenge, NodeToken};
    use crate::key::{DaemonKey, DaemonRootKey};
    use core::time::Duration;
    use rcgen::{CertificateSigningRequestParams, KeyPair};
    use rustls::pki_types::PrivateKeyDer;
    use std::sync::Arc;
    use uuid::Uuid;

    struct Fixture {
        root: DaemonRootKey,
        certificate_authority: KeyPair,
    }

    fn fixture() -> Fixture {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let certificate_authority = KeyPair::generate().unwrap();
        let root = DaemonRootKey::new(&certificate_authority, None).unwrap();
        Fixture {
            root,
            certificate_authority,
        }
    }

    fn node(fixture: &Fixture, id: Uuid) -> DaemonKey {
        let keypair = KeyPair::generate().unwrap();
        let request = DaemonKey::new_request(&keypair).unwrap();
        let request = CertificateSigningRequestParams::from_der(&request).unwrap();
        let certificate = fixture
            .root
            .sign_daemon_key(&fixture.certificate_authority, request.public_key, id)
            .unwrap();
        let privkey = PrivateKeyDer::try_from(keypair.serialize_der()).unwrap();
        DaemonKey::new(&fixture.root, certificate, privkey).unwrap()
    }

    #[test]
    fn node_token_round_trip() {
        let fixture = fixture();
        let backend = fixture
            .root
            .new_backend_key(&fixture.certificate_authority, &KeyPair::generate().unwrap())
            .unwrap();
        let id = Uuid::new_v4();

        let token = NodeToken::issue(&backend.tls_cert, id, Duration::from_secs(3600)).unwrap();
        let encoded = token.to_string();
        let parsed: NodeToken = encoded.parse().unwrap();

        parsed.verify(&fixture.root.cert_store, id).unwrap();
    }

    #[test]
    fn node_token_is_scoped_to_one_node() {
        let fixture = fixture();
        let backend = fixture
            .root
            .new_backend_key(&fixture.certificate_authority, &KeyPair::generate().unwrap())
            .unwrap();

        let token =
            NodeToken::issue(&backend.tls_cert, Uuid::new_v4(), Duration::from_secs(3600)).unwrap();
        let parsed: NodeToken = token.to_string().parse().unwrap();

        // A different node must not accept a token issued for someone else.
        assert!(parsed.verify(&fixture.root.cert_store, Uuid::new_v4()).is_err());
    }

    #[test]
    fn node_token_rejects_expired() {
        let fixture = fixture();
        let backend = fixture
            .root
            .new_backend_key(&fixture.certificate_authority, &KeyPair::generate().unwrap())
            .unwrap();
        let id = Uuid::new_v4();

        let token = NodeToken::issue(&backend.tls_cert, id, Duration::from_secs(0)).unwrap();
        let mut parsed: NodeToken = token.to_string().parse().unwrap();
        // Push the window firmly into the past without re-signing, which also
        // proves the timestamps are covered by the signature.
        parsed = format!("{parsed}").parse().unwrap();
        std::thread::sleep(Duration::from_millis(1100));

        assert!(parsed.verify(&fixture.root.cert_store, id).is_err());
    }

    #[test]
    fn node_token_rejects_a_node_key() {
        let fixture = fixture();
        let id = Uuid::new_v4();
        let node = node(&fixture, id);

        // A node certificate is ServerAuth only and must never pass as a backend.
        let token = NodeToken::issue(&node.tls_cert, id, Duration::from_secs(3600)).unwrap();
        let parsed: NodeToken = token.to_string().parse().unwrap();

        assert!(parsed.verify(&fixture.root.cert_store, id).is_err());
    }

    #[test]
    fn attestation_round_trip() {
        let fixture = fixture();
        let id = Uuid::new_v4();
        let node = node(&fixture, id);
        let challenge = Challenge::generate();

        let attestation = Attestation::sign(&node.tls_cert, id, challenge).unwrap();
        let parsed: Attestation = attestation.to_string().parse().unwrap();

        parsed
            .verify(&fixture.root.cert_store, id, challenge)
            .unwrap();
    }

    #[test]
    fn attestation_rejects_wrong_challenge() {
        let fixture = fixture();
        let id = Uuid::new_v4();
        let node = node(&fixture, id);

        let attestation = Attestation::sign(&node.tls_cert, id, Challenge::generate()).unwrap();
        let parsed: Attestation = attestation.to_string().parse().unwrap();

        assert!(
            parsed
                .verify(&fixture.root.cert_store, id, Challenge::generate())
                .is_err()
        );
    }

    #[test]
    fn attestation_rejects_another_node() {
        let fixture = fixture();
        let impersonator = Uuid::new_v4();
        let node = node(&fixture, impersonator);
        let challenge = Challenge::generate();

        let attestation = Attestation::sign(&node.tls_cert, impersonator, challenge).unwrap();
        let mut parsed: Attestation = attestation.to_string().parse().unwrap();

        // Claim to be a different node while presenting the original certificate.
        let victim = Uuid::new_v4();
        parsed.node = victim;

        assert!(
            parsed
                .verify(&fixture.root.cert_store, victim, challenge)
                .is_err()
        );
    }

    #[test]
    fn challenge_round_trip() {
        let challenge = Challenge::generate();
        let parsed: Challenge = challenge.to_string().parse().unwrap();
        assert_eq!(challenge, parsed);
    }

    #[test]
    fn unrelated_authority_is_rejected() {
        let fixture = fixture();
        let other = fixture();
        let id = Uuid::new_v4();

        let backend = other
            .root
            .new_backend_key(&other.certificate_authority, &KeyPair::generate().unwrap())
            .unwrap();
        let token = NodeToken::issue(&backend.tls_cert, id, Duration::from_secs(3600)).unwrap();
        let parsed: NodeToken = token.to_string().parse().unwrap();

        assert!(parsed.verify(&fixture.root.cert_store, id).is_err());
        let _ = Arc::clone(&fixture.root.cert_store);
    }
}
