use ed25519_dalek::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey};
use ed25519_dalek::{SignatureError, SigningKey, VerifyingKey};
use pem::{Pem, PemError};
use rcgen::{
    BasicConstraints, CertificateParams, CertificateSigningRequestParams, DistinguishedName,
    DnType, ExtendedKeyUsagePurpose, Ia5String, IsCa, KeyPair, KeyUsagePurpose, PublicKey, SanType,
};
use rustls::pki_types::pem::SectionKind;
use rustls::pki_types::{
    CertificateDer, CertificateSigningRequestDer, DnsName, PrivateKeyDer, PrivatePkcs1KeyDer,
    PrivatePkcs8KeyDer, PrivateSec1KeyDer, ServerName, UnixTime,
};
use rustls::sign::CertifiedKey;
use rustls::{DigitallySignedStruct, Error, RootCertStore, SignatureScheme};
use rustls_pemfile::Item;
use std::fmt::Debug;
use std::io;
use std::str::FromStr;
use std::sync::Arc;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;
use x509_parser::certificate::X509Certificate;

pub use rcgen;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
pub use x509_parser;

pub struct DaemonRootKey {
    pub tls_ca: CertificateDer<'static>,
    pub cert_store: Arc<RootCertStore>,
    pub user_key: Option<VerifyingKey>,
}

pub struct DaemonKey {
    pub id: Uuid,
    pub privkey: PrivateKeyDer<'static>,
    pub cert: Vec<CertificateDer<'static>>,
    pub tls_cert: Arc<CertifiedKey>,
}

pub struct BackendKey {
    pub privkey: PrivateKeyDer<'static>,
    pub cert: Vec<CertificateDer<'static>>,
    pub tls_cert: Arc<CertifiedKey>,
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonKeyError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("pem error: {0}")]
    Pem(#[from] PemError),
    #[error("der error: {0}")]
    Der(&'static str),
    #[error("pkcs8 error: {0}")]
    Pkcs8(#[from] ed25519_dalek::pkcs8::Error),
    #[error("rustls error: {0}")]
    Rustls(#[from] rustls::Error),
    #[error("x509 parsing error: {0}")]
    X509(#[from] asn1_rs::Err<x509_parser::error::X509Error>),
    #[error("private key not present")]
    PrivateKeyMissing,
    #[error("invalid key")]
    Invalid,
    #[error("unable to generate or parse key")]
    Rcgen(#[from] rcgen::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum BackendKeyError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("der error: {0}")]
    Der(&'static str),
    #[error("rustls error: {0}")]
    Rustls(#[from] rustls::Error),
    #[error("x509 parsing error: {0}")]
    X509(#[from] asn1_rs::Err<x509_parser::error::X509Error>),
    #[error("private key not present")]
    PrivateKeyMissing,
    #[error("invalid key")]
    Invalid,
    #[error("unable to generate or parse key")]
    Rcgen(#[from] rcgen::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonRootKeyError {
    #[error("der error: {0}")]
    Der(&'static str),
    #[error("pem error: {0}")]
    Pem(#[from] PemError),
    #[error("pkcs8 error: {0}")]
    Pkcs8(#[from] ed25519_dalek::pkcs8::Error),
    #[error("pkcs8 spki error: {0}")]
    Pkcs8Spki(#[from] ed25519_dalek::pkcs8::spki::Error),
    #[error("user key error: {0}")]
    UserKey(#[from] SignatureError),
    #[error("rustls error: {0}")]
    Rustls(#[from] rustls::Error),
    #[error("x509 parsing error: {0}")]
    X509(#[from] asn1_rs::Err<x509_parser::error::X509Error>),
    #[error("invalid key")]
    Invalid,
    #[error("unable to generate or parse key")]
    Rcgen(#[from] rcgen::Error),
}

impl DaemonRootKey {
    pub fn new(
        keypair: &KeyPair,
        user_key: Option<VerifyingKey>,
    ) -> Result<Self, DaemonRootKeyError> {
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);

        let mut ca_name = DistinguishedName::new();
        ca_name.push(DnType::CommonName, crate::DAEMON_ROOT_KEY_COMMON_NAME);

        params.distinguished_name = ca_name;

        params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        params.key_usages.push(KeyUsagePurpose::KeyCertSign);
        params.key_usages.push(KeyUsagePurpose::CrlSign);

        params.not_after = OffsetDateTime::now_utc()
            .checked_add(Duration::days(365 * 64))
            .unwrap();
        params.not_before = OffsetDateTime::now_utc();

        let cert = params.self_signed(keypair)?;

        let tls_ca = cert.der().clone();

        let mut cert_store = RootCertStore::empty();
        cert_store.add(tls_ca.clone())?;

        Ok(Self {
            tls_ca,
            cert_store: Arc::new(cert_store),
            user_key,
        })
    }

    pub fn x509(&self) -> X509Certificate<'_> {
        let (_, x509) = x509_parser::parse_x509_certificate(&self.tls_ca)
            .expect("Unable to reproduce successful x509 parse");
        x509
    }

    pub fn parse(
        data: impl AsRef<[u8]>,
    ) -> Result<(DaemonRootKey, Option<KeyPair>, Option<SigningKey>), DaemonRootKeyError> {
        let mut tls_ca = None;
        let mut public_user_key = None;
        let mut user_key = None;
        let mut privkey = None;

        let pem = pem::parse_many(data)?;

        for cert in pem {
            let section = SectionKind::try_from(cert.tag().as_bytes());
            match (cert.tag(), section) {
                ("USER KEY", _) => {
                    public_user_key = Some(VerifyingKey::from_public_key_der(cert.contents())?);
                }
                ("PRIVATE USER KEY", _) => {
                    let key = SigningKey::from_pkcs8_der(cert.contents())?;
                    public_user_key = Some(key.verifying_key());
                    user_key = Some(key);
                }
                (_, Ok(section)) => {
                    use SectionKind::*;
                    let contents = cert.into_contents();

                    match section {
                        Certificate => {
                            tls_ca = Some(CertificateDer::from(contents));
                        }
                        PrivateKey => {
                            privkey = Some(
                                PrivatePkcs8KeyDer::from(contents)
                                    .secret_pkcs8_der()
                                    .to_vec(),
                            );
                        }
                        RsaPrivateKey => {
                            privkey = Some(
                                PrivatePkcs1KeyDer::from(contents)
                                    .secret_pkcs1_der()
                                    .to_vec(),
                            );
                        }
                        EcPrivateKey => {
                            privkey =
                                Some(PrivateSec1KeyDer::from(contents).secret_sec1_der().to_vec());
                        }
                        _ => {}
                    }
                }
                (_, Err(_)) => {
                    // Ignore the tag for now
                }
            }
        }

        let tls_ca = tls_ca.ok_or(rustls::Error::NoCertificatesPresented)?;

        let (_, x509) = x509_parser::parse_x509_certificate(&tls_ca)?;
        if !x509.is_ca() {
            return Err(DaemonRootKeyError::Invalid);
        }
        let mut cert_store = RootCertStore::empty();
        cert_store.add(tls_ca.clone())?;

        let mut keypair = None;
        if let Some(privkey) = privkey {
            let privkey = PrivateKeyDer::try_from(privkey).map_err(DaemonRootKeyError::Der)?;
            keypair = Some(KeyPair::try_from(&privkey)?);
        }

        Ok((
            DaemonRootKey {
                tls_ca,
                cert_store: Arc::new(cert_store),
                user_key: public_user_key,
            },
            keypair,
            user_key,
        ))
    }

    pub fn dump(&self, keypair: Option<&KeyPair>, user_key: Option<&SigningKey>) -> String {
        let mut out = String::new();
        if let Some(keypair) = keypair {
            let pem = Pem::new("PRIVATE KEY", keypair.serialize_der());
            out += &pem::encode(&pem);
        }

        let pem = Pem::new("CERTIFICATE", self.tls_ca.to_vec());
        out += &pem::encode(&pem);
        out += "\n";
        if let Some(public_user_key) = &self.user_key {
            match user_key {
                None => {
                    let pem = Pem::new(
                        "USER KEY",
                        public_user_key.to_public_key_der().unwrap().as_bytes(),
                    );
                    out += &pem::encode(&pem);
                    out += "\n";
                }
                Some(user_key) => {
                    let pem = Pem::new(
                        "PRIVATE USER KEY",
                        user_key.to_pkcs8_der().unwrap().as_bytes(),
                    );
                    out += &pem::encode(&pem);
                    out += "\n";
                }
            }
        }
        out
    }

    pub fn sign_daemon_key(
        &self,
        root_privkey: &KeyPair,
        public_key: PublicKey,
        identifier: Uuid,
    ) -> Result<CertificateDer<'static>, rcgen::Error> {
        let mut params = CertificateParams::default();
        params
            .distinguished_name
            .push(DnType::CommonName, format!("node-{}", identifier));

        params
            .subject_alt_names
            .push(SanType::DnsName(Ia5String::try_from(format!(
                "node-{}",
                identifier
            ))?));

        params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        params
            .extended_key_usages
            .push(ExtendedKeyUsagePurpose::ServerAuth);
        params.use_authority_key_identifier_extension = true;

        let csr_params = CertificateSigningRequestParams { public_key, params };

        let root_cert = CertificateParams::from_ca_cert_der(&self.tls_ca)?;
        let root_cert = root_cert.self_signed(root_privkey)?;

        let signed = csr_params.signed_by(&root_cert, root_privkey)?;

        Ok(signed.der().clone())
    }

    pub fn new_backend_key(
        &self,
        root_keypair: &KeyPair,
        keypair: &KeyPair,
    ) -> Result<BackendKey, BackendKeyError> {
        let mut params = CertificateParams::default();

        let mut backend_name = DistinguishedName::new();
        backend_name.push(DnType::CommonName, crate::DAEMON_BACKEND_KEY_COMMON_NAME);

        params.distinguished_name = backend_name;

        params.key_usages.push(KeyUsagePurpose::DigitalSignature);
        params
            .extended_key_usages
            .push(ExtendedKeyUsagePurpose::ClientAuth);
        params.use_authority_key_identifier_extension = true;

        params.not_after = OffsetDateTime::now_utc()
            .checked_add(Duration::days(365 * 64))
            .unwrap();
        params.not_before = OffsetDateTime::now_utc();

        let root_cert = CertificateParams::from_ca_cert_der(&self.tls_ca)?;
        let root_cert = root_cert.self_signed(root_keypair)?;

        let cert = params.signed_by(keypair, &root_cert, root_keypair)?;

        let privkey =
            PrivateKeyDer::try_from(keypair.serialize_der()).map_err(BackendKeyError::Der)?;
        let cert = vec![cert.der().clone()];

        let mut tls_cert = cert.clone();
        tls_cert.push(self.tls_ca.clone());

        let tls_cert = CertifiedKey::from_der(
            tls_cert,
            privkey.clone_key(),
            &rustls::crypto::aws_lc_rs::default_provider(),
        )?;

        Ok(BackendKey {
            privkey,
            cert,
            tls_cert: Arc::new(tls_cert),
        })
    }
}

impl BackendKey {
    pub fn x509(&self) -> X509Certificate<'_> {
        let (_, x509) = x509_parser::parse_x509_certificate(
            self.tls_cert
                .end_entity_cert()
                .expect("Unable to reproduce successful end entity cert search"),
        )
        .expect("Unable to reproduce successful x509 parse");
        x509
    }

    pub fn parse(
        root: &DaemonRootKey,
        data: impl AsRef<[u8]>,
    ) -> Result<BackendKey, BackendKeyError> {
        #[allow(clippy::useless_asref)] // False positive
        let cert = rustls_pemfile::certs(&mut data.as_ref()).collect::<Result<Vec<_>, _>>()?;
        let mut tls_certificate = cert.clone();
        tls_certificate.push(root.tls_ca.clone());

        // Check the entire PEM file for the key in case it is not first section
        #[allow(clippy::useless_asref)] // False positive
        let mut tls_privkeys: Vec<Vec<u8>> = rustls_pemfile::read_all(&mut data.as_ref())
            .filter_map(|i| match i.ok()? {
                Item::Sec1Key(key) => Some(key.secret_sec1_der().to_vec()),
                Item::Pkcs1Key(key) => Some(key.secret_pkcs1_der().to_vec()),
                Item::Pkcs8Key(key) => Some(key.secret_pkcs8_der().to_vec()),
                _ => None,
            })
            .collect();

        let privkey = tls_privkeys
            .pop()
            .ok_or(BackendKeyError::PrivateKeyMissing)?;
        let privkey = PrivateKeyDer::try_from(privkey).map_err(BackendKeyError::Der)?;

        let tls_cert = CertifiedKey::from_der(
            tls_certificate,
            privkey.clone_key(),
            &rustls::crypto::aws_lc_rs::default_provider(),
        )?;

        tls_cert.keys_match()?;

        let (_, x509) = x509_parser::parse_x509_certificate(tls_cert.end_entity_cert()?)?;
        let key_usage = x509
            .extended_key_usage()
            .map_err(|_| BackendKeyError::Invalid)?
            .ok_or(BackendKeyError::Invalid)?;
        if !key_usage.value.client_auth {
            return Err(BackendKeyError::Invalid);
        }

        Ok(BackendKey {
            privkey,
            cert,
            tls_cert: Arc::new(tls_cert),
        })
    }

    pub fn dump(&self) -> String {
        let pem = Pem::new("PRIVATE KEY", self.privkey.secret_der().to_vec());
        let mut out = pem::encode(&pem);
        out += "\n";
        for cert in &self.cert {
            let pem = Pem::new("CERTIFICATE", cert.to_vec());
            out += &pem::encode(&pem);
            out += "\n";
        }

        out
    }
}

impl DaemonKey {
    pub fn new(
        root: &DaemonRootKey,
        signed: CertificateDer<'static>,
        privkey: PrivateKeyDer<'static>,
    ) -> Result<DaemonKey, DaemonKeyError> {
        let tls_certificate = vec![signed.clone(), root.tls_ca.clone()];

        let tls_cert = CertifiedKey::from_der(
            tls_certificate,
            privkey.clone_key(),
            &rustls::crypto::aws_lc_rs::default_provider(),
        )?;

        tls_cert.keys_match()?;

        let (_, x509) = x509_parser::parse_x509_certificate(tls_cert.end_entity_cert()?)?;

        let common_name = x509
            .subject
            .iter_common_name()
            .next()
            .ok_or(DaemonKeyError::Invalid)?;
        let common_name = common_name.as_str().map_err(asn1_rs::Err::Error)?;

        let id = common_name
            .strip_prefix("node-")
            .ok_or(DaemonKeyError::Invalid)?;

        Ok(DaemonKey {
            id: Uuid::from_str(id).map_err(|_| DaemonKeyError::Invalid)?,
            privkey,
            cert: vec![signed],
            tls_cert: Arc::new(tls_cert),
        })
    }

    pub fn new_temp(keypair: &KeyPair) -> Result<DaemonKey, DaemonKeyError> {
        let id = Uuid::nil();

        let mut distinguished_name = DistinguishedName::new();
        distinguished_name.push(DnType::CommonName, format!("node-{}", id));

        let mut params = CertificateParams::default();
        params.distinguished_name = distinguished_name;

        params
            .subject_alt_names
            .push(SanType::DnsName(Ia5String::try_from(format!(
                "node-{}",
                id
            ))?));

        params.not_after = OffsetDateTime::now_utc()
            .checked_add(Duration::minutes(30))
            .unwrap();
        params.not_before = OffsetDateTime::now_utc();

        let request = params.self_signed(keypair)?;

        let cert = request.der().clone();
        let privkey =
            PrivateKeyDer::try_from(keypair.serialize_der()).map_err(DaemonKeyError::Der)?;

        let tls_cert = CertifiedKey::from_der(
            vec![cert.clone()],
            privkey.clone_key(),
            &rustls::crypto::aws_lc_rs::default_provider(),
        )?;

        Ok(DaemonKey {
            id,
            privkey,
            cert: vec![cert],
            tls_cert: Arc::new(tls_cert),
        })
    }

    pub fn new_request(
        keypair: &KeyPair,
    ) -> Result<CertificateSigningRequestDer<'static>, rcgen::Error> {
        let mut distinguished_name = DistinguishedName::new();
        distinguished_name.push(DnType::CommonName, "node");

        let mut params = CertificateParams::default();
        params.distinguished_name = distinguished_name;

        params.not_after = OffsetDateTime::now_utc()
            .checked_add(Duration::days(365 * 64))
            .unwrap();
        params.not_before = OffsetDateTime::now_utc();

        let request = params.serialize_request(keypair)?;
        Ok(request.der().clone())
    }

    pub fn x509(&self) -> X509Certificate<'_> {
        let (_, x509) = x509_parser::parse_x509_certificate(
            self.tls_cert
                .end_entity_cert()
                .expect("Unable to reproduce successful end entity cert search"),
        )
        .expect("Unable to reproduce successful x509 parse");
        x509
    }

    pub fn parse(root: &DaemonRootKey, data: &[u8]) -> Result<DaemonKey, DaemonKeyError> {
        #[allow(clippy::useless_asref)] // False positive
        let cert = rustls_pemfile::certs(&mut data.as_ref()).collect::<Result<Vec<_>, _>>()?;
        let mut tls_certificate = cert.clone();
        tls_certificate.push(root.tls_ca.clone());

        // Check the entire PEM file for the key in case it is not first section
        #[allow(clippy::useless_asref)] // False positive
        let mut tls_privkeys: Vec<Vec<u8>> = rustls_pemfile::read_all(&mut data.as_ref())
            .filter_map(|i| match i.ok()? {
                Item::Sec1Key(key) => Some(key.secret_sec1_der().to_vec()),
                Item::Pkcs1Key(key) => Some(key.secret_pkcs1_der().to_vec()),
                Item::Pkcs8Key(key) => Some(key.secret_pkcs8_der().to_vec()),
                _ => None,
            })
            .collect();

        let privkey = tls_privkeys
            .pop()
            .ok_or(DaemonKeyError::PrivateKeyMissing)?;
        let privkey = PrivateKeyDer::try_from(privkey).map_err(DaemonKeyError::Der)?;

        let tls_cert = CertifiedKey::from_der(
            tls_certificate,
            privkey.clone_key(),
            &rustls::crypto::aws_lc_rs::default_provider(),
        )?;

        tls_cert.keys_match()?;

        let (_, x509) = x509_parser::parse_x509_certificate(tls_cert.end_entity_cert()?)?;

        let key_usage = x509
            .extended_key_usage()
            .map_err(|_| DaemonKeyError::Invalid)?
            .ok_or(DaemonKeyError::Invalid)?;
        if !key_usage.value.server_auth {
            return Err(DaemonKeyError::Invalid);
        }

        let common_name = x509
            .subject
            .iter_common_name()
            .next()
            .ok_or(DaemonKeyError::Invalid)?;
        let common_name = common_name.as_str().map_err(asn1_rs::Err::Error)?;

        let id = common_name
            .strip_prefix("node-")
            .ok_or(DaemonKeyError::Invalid)?;

        Ok(DaemonKey {
            id: Uuid::from_str(id).map_err(|_| DaemonKeyError::Invalid)?,
            privkey,
            cert,
            tls_cert: Arc::new(tls_cert),
        })
    }

    pub fn dump(&self) -> String {
        let pem = Pem::new("PRIVATE KEY", self.privkey.secret_der().to_vec());
        let mut out = pem::encode(&pem);
        out += "\n";
        for cert in &self.cert {
            let pem = Pem::new("CERTIFICATE", cert.to_vec());
            out += &pem::encode(&pem);
            out += "\n";
        }

        out
    }
}

#[derive(Debug)]
pub struct DaemonKeyVerifier(pub Uuid, pub Arc<dyn ServerCertVerifier>);

impl ServerCertVerifier for DaemonKeyVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        self.1.verify_server_cert(
            end_entity,
            intermediates,
            &ServerName::DnsName(DnsName::try_from(format!("node-{}", self.0)).unwrap()),
            ocsp_response,
            now,
        )
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.1.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.1.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.1.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use crate::key::{BackendKey, DaemonKey, DaemonRootKey};
    use ed25519_dalek::SigningKey;
    use ed25519_dalek::ed25519::signature::rand_core::OsRng;
    use rcgen::{CertificateSigningRequestParams, KeyPair};
    use rustls::pki_types::PrivateKeyDer;
    use uuid::Uuid;

    #[test]
    fn keys() {
        /*rustls::crypto::ring::default_provider()
        .install_default()
        .unwrap();*/

        // Root
        let ca = KeyPair::generate().unwrap();

        let user_key = SigningKey::generate(&mut OsRng);
        let public_user_key = user_key.verifying_key();

        let root = DaemonRootKey::new(&ca, Some(public_user_key)).unwrap();

        let root_dumped_priv = root.dump(Some(&ca), Some(&user_key));
        let root_parsed_priv = DaemonRootKey::parse(&root_dumped_priv).unwrap();
        assert_eq!(
            root_parsed_priv
                .1
                .map(|pair| pair.public_key_raw().to_vec()),
            Some(ca.public_key_raw().to_vec())
        );
        assert_eq!(root_parsed_priv.2, Some(user_key));

        let root_dumped = root_parsed_priv.0.dump(None, None);

        let root_parsed = DaemonRootKey::parse(&root_dumped).unwrap();

        assert!(root_parsed.1.is_none());
        assert!(root_parsed.2.is_none());
        assert_eq!(root.user_key, root_parsed.0.user_key);
        assert_eq!(root.tls_ca, root_parsed.0.tls_ca);

        let x509 = root_parsed.0.x509();
        assert!(x509.is_ca());

        // Backend key
        let backend_keypair = KeyPair::generate().unwrap();
        let backend_key = root.new_backend_key(&ca, &backend_keypair).unwrap();
        let backend_dumped = backend_key.dump();
        let backend_parsed = BackendKey::parse(&root, backend_dumped.as_bytes()).unwrap();

        assert_eq!(backend_parsed.privkey, backend_key.privkey);
        assert_eq!(backend_parsed.tls_cert.cert, backend_key.tls_cert.cert);
        assert_eq!(backend_parsed.x509(), backend_key.x509());

        // Daemon key
        let node_id = Uuid::new_v4();

        let node_keypair = KeyPair::generate().unwrap();
        let node_req = DaemonKey::new_request(&node_keypair).unwrap();

        let node_req = CertificateSigningRequestParams::from_der(&node_req).unwrap();

        let node_cert = root
            .sign_daemon_key(&ca, node_req.public_key, node_id)
            .unwrap();
        let node_privkey = PrivateKeyDer::try_from(node_keypair.serialize_der()).unwrap();

        let node_key = DaemonKey::new(&root, node_cert, node_privkey).unwrap();

        assert_eq!(node_key.id, node_id);

        let node_key_dumped = node_key.dump();
        let node_key_parsed = DaemonKey::parse(&root, node_key_dumped.as_bytes()).unwrap();

        assert_eq!(node_key_parsed.privkey, node_key.privkey);
        assert_eq!(node_key_parsed.tls_cert.cert, node_key.tls_cert.cert);
        assert_eq!(node_key_parsed.x509(), node_key.x509());
    }
}
