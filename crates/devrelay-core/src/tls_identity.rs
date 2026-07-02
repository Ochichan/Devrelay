//! Fabric X.509 material for the mTLS control plane.
//!
//! The control transport authenticates devices with X.509 certificates while
//! DevRelay's application-level trust lives in fabric-root-signed
//! [`crate::DeviceCertificate`] records. This module bridges the two layers:
//!
//! - The fabric X.509 CA key is derived from the fabric root secret with a
//!   domain-separated KDF, so the root signing key never signs raw DER.
//! - A device's TLS leaf certificate carries the device's ed25519 signing
//!   public key, the same key recorded in its `DeviceCertificate`. Servers and
//!   clients bind the TLS channel to the application peer by comparing the
//!   peer leaf's SubjectPublicKeyInfo against the validated certificate.
//! - All certificate parameters are deterministic (fixed serial policy, fixed
//!   validity range, names derived from fabric and device ids), so every
//!   holder of the fabric root secret regenerates byte-identical DER. Leaf
//!   lifetime is intentionally not enforced at the X.509 layer; expiry and
//!   revocation are enforced per request against the `DeviceCertificate`.

use crate::{DevRelayError, Result, RustlsIdentity};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose, PKCS_ED25519, SerialNumber, date_time_ymd,
};
use rustls::pki_types::PrivatePkcs8KeyDer;

/// SAN and rustls `ServerName` used by every control-plane certificate.
///
/// Device authentication does not rely on TLS names: both sides bind the
/// channel to the peer's ed25519 key after the handshake.
pub const DEVRELAY_CONTROL_SERVER_NAME: &str = "devrelay-control";

const FABRIC_TLS_CA_KEY_CONTEXT: &str = "devrelay 2026-06 fabric x509 ca key v1";
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];
const ED25519_PKCS8_V1_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

pub(crate) fn fabric_tls_ca_der(root_secret: &[u8; 32], fabric_id: &str) -> Result<Vec<u8>> {
    let ca_key = fabric_ca_key_pair(root_secret)?;
    let params = fabric_ca_params(fabric_id)?;
    let certificate = params.self_signed(&ca_key).map_err(|err| {
        DevRelayError::Config(format!("failed to self-sign fabric TLS CA: {err}"))
    })?;
    Ok(certificate.der().as_ref().to_vec())
}

pub(crate) fn issue_device_tls_leaf_der(
    root_secret: &[u8; 32],
    fabric_id: &str,
    device_id: &str,
    device_signing_public_key: &[u8; 32],
) -> Result<Vec<u8>> {
    let ca_key = fabric_ca_key_pair(root_secret)?;
    let ca_certificate = fabric_ca_params(fabric_id)?
        .self_signed(&ca_key)
        .map_err(|err| {
            DevRelayError::Config(format!("failed to self-sign fabric TLS CA: {err}"))
        })?;

    // rcgen only signs certificates for key pairs it holds, so the leaf public
    // key is injected through a remote key-pair shim; the private half stays
    // on the subject device.
    let leaf_public = KeyPair::from_remote(Box::new(UnsignableEd25519PublicKey(
        spki_from_public_key(device_signing_public_key),
    )))
    .map_err(|err| DevRelayError::Config(format!("invalid device TLS public key: {err}")))?;
    let certificate = device_leaf_params(device_id, device_signing_public_key)?
        .signed_by(&leaf_public, &ca_certificate, &ca_key)
        .map_err(|err| {
            DevRelayError::Config(format!("failed to issue device TLS certificate: {err}"))
        })?;
    Ok(certificate.der().as_ref().to_vec())
}

pub(crate) fn device_tls_identity(
    root_secret: &[u8; 32],
    fabric_id: &str,
    device_id: &str,
    device_signing_secret: &[u8; 32],
) -> Result<RustlsIdentity> {
    let signing_key = ed25519_dalek::SigningKey::from_bytes(device_signing_secret);
    let public_key = signing_key.verifying_key().to_bytes();
    let leaf_der = issue_device_tls_leaf_der(root_secret, fabric_id, device_id, &public_key)?;
    Ok(RustlsIdentity {
        cert_chain_der: vec![leaf_der],
        private_key_pkcs8_der: ed25519_seed_to_pkcs8_der(device_signing_secret),
    })
}

/// Extracts the single ed25519 SubjectPublicKeyInfo key from a certificate.
///
/// Control-plane certificates are only inspected after the TLS layer verified
/// them against the fabric CA, and every certificate this fabric issues holds
/// exactly one ed25519 SPKI. Anything else is rejected.
pub fn extract_single_ed25519_spki(certificate_der: &[u8]) -> Result<[u8; 32]> {
    let mut found: Option<[u8; 32]> = None;
    let mut offset = 0;
    while offset + ED25519_SPKI_PREFIX.len() + 32 <= certificate_der.len() {
        if certificate_der[offset..offset + ED25519_SPKI_PREFIX.len()] == ED25519_SPKI_PREFIX {
            let start = offset + ED25519_SPKI_PREFIX.len();
            let mut key = [0u8; 32];
            key.copy_from_slice(&certificate_der[start..start + 32]);
            if found.is_some() {
                return Err(DevRelayError::Config(
                    "certificate contains more than one ed25519 public key".to_string(),
                ));
            }
            found = Some(key);
            offset = start + 32;
        } else {
            offset += 1;
        }
    }
    found.ok_or_else(|| {
        DevRelayError::Config("certificate does not contain an ed25519 public key".to_string())
    })
}

pub(crate) fn ed25519_seed_to_pkcs8_der(seed: &[u8; 32]) -> Vec<u8> {
    let mut der = Vec::with_capacity(ED25519_PKCS8_V1_PREFIX.len() + seed.len());
    der.extend_from_slice(&ED25519_PKCS8_V1_PREFIX);
    der.extend_from_slice(seed);
    der
}

fn fabric_ca_key_pair(root_secret: &[u8; 32]) -> Result<KeyPair> {
    let ca_seed = blake3::derive_key(FABRIC_TLS_CA_KEY_CONTEXT, root_secret);
    let pkcs8 = ed25519_seed_to_pkcs8_der(&ca_seed);
    KeyPair::from_pkcs8_der_and_sign_algo(&PrivatePkcs8KeyDer::from(pkcs8), &PKCS_ED25519)
        .map_err(|err| DevRelayError::Config(format!("invalid fabric TLS CA key: {err}")))
}

fn fabric_ca_params(fabric_id: &str) -> Result<CertificateParams> {
    let mut params = CertificateParams::new(Vec::<String>::new())
        .map_err(|err| DevRelayError::Config(format!("invalid fabric TLS CA params: {err}")))?;
    params.distinguished_name = DistinguishedName::new();
    params.distinguished_name.push(
        DnType::CommonName,
        format!("DevRelay Fabric CA {fabric_id}"),
    );
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.serial_number = Some(SerialNumber::from(vec![0x01]));
    params.not_before = date_time_ymd(1975, 1, 1);
    params.not_after = date_time_ymd(4096, 1, 1);
    Ok(params)
}

fn device_leaf_params(
    device_id: &str,
    device_signing_public_key: &[u8; 32],
) -> Result<CertificateParams> {
    let mut params = CertificateParams::new(vec![DEVRELAY_CONTROL_SERVER_NAME.to_string()])
        .map_err(|err| DevRelayError::Config(format!("invalid device TLS params: {err}")))?;
    params.distinguished_name = DistinguishedName::new();
    params
        .distinguished_name
        .push(DnType::CommonName, device_id.to_string());
    params.is_ca = IsCa::ExplicitNoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ClientAuth,
        ExtendedKeyUsagePurpose::ServerAuth,
    ];
    params.use_authority_key_identifier_extension = true;
    let mut serial = blake3::hash(device_signing_public_key).as_bytes()[..16].to_vec();
    serial[0] &= 0x7f;
    params.serial_number = Some(SerialNumber::from(serial));
    params.not_before = date_time_ymd(1975, 1, 1);
    params.not_after = date_time_ymd(4096, 1, 1);
    Ok(params)
}

fn spki_from_public_key(public_key: &[u8; 32]) -> Vec<u8> {
    let mut spki = Vec::with_capacity(ED25519_SPKI_PREFIX.len() + public_key.len());
    spki.extend_from_slice(&ED25519_SPKI_PREFIX);
    spki.extend_from_slice(public_key);
    spki
}

/// Public-key-only shim so the fabric CA can issue certificates for peer
/// devices without holding their private keys.
struct UnsignableEd25519PublicKey(Vec<u8>);

impl rcgen::RemoteKeyPair for UnsignableEd25519PublicKey {
    fn public_key(&self) -> &[u8] {
        // rcgen expects the raw subject public key, not the full SPKI.
        &self.0[ED25519_SPKI_PREFIX.len()..]
    }

    fn sign(&self, _msg: &[u8]) -> std::result::Result<Vec<u8>, rcgen::Error> {
        Err(rcgen::Error::RemoteKeyError)
    }

    fn algorithm(&self) -> &'static rcgen::SignatureAlgorithm {
        &PKCS_ED25519
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DevRelayHome, FabricIdentityStore, LocalConfig, build_rustls_server_config};

    fn test_store() -> (tempfile::TempDir, FabricIdentityStore, LocalConfig) {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path());
        let config = LocalConfig::new_for_local_device();
        let store = FabricIdentityStore::new(home);
        store.open_or_create(&config).unwrap();
        (temp, store, config)
    }

    #[test]
    fn fabric_tls_ca_and_leaf_are_deterministic_per_fabric() {
        let (_temp, store, config) = test_store();

        let first_ca = store.fabric_tls_ca_der().unwrap();
        let second_ca = store.fabric_tls_ca_der().unwrap();
        let first_leaf = store.device_tls_identity(&config.device_id).unwrap();
        let second_leaf = store.device_tls_identity(&config.device_id).unwrap();

        assert_eq!(first_ca, second_ca);
        assert_eq!(first_leaf, second_leaf);
        assert_ne!(first_ca, first_leaf.cert_chain_der[0]);
    }

    #[test]
    fn device_tls_identity_carries_device_signing_key() {
        let (_temp, store, config) = test_store();
        let bundle = store.public_bundle_from_store(&config).unwrap();

        let identity = store.device_tls_identity(&config.device_id).unwrap();
        let leaf_key = extract_single_ed25519_spki(&identity.cert_chain_der[0]).unwrap();

        assert_eq!(
            bundle.device.signing_public_key_hex,
            leaf_key.iter().fold(String::new(), |mut acc, byte| {
                acc.push_str(&format!("{byte:02x}"));
                acc
            })
        );
    }

    #[test]
    fn issued_peer_leaf_carries_peer_key_and_builds_mtls_config() {
        let (_temp, store, config) = test_store();
        let peer_secret = [7u8; 32];
        let peer_public = ed25519_dalek::SigningKey::from_bytes(&peer_secret)
            .verifying_key()
            .to_bytes();
        let peer_public_hex = peer_public.iter().fold(String::new(), |mut acc, byte| {
            acc.push_str(&format!("{byte:02x}"));
            acc
        });

        let peer_leaf = store
            .issue_peer_tls_certificate_der("peer-device", &peer_public_hex)
            .unwrap();
        let extracted = extract_single_ed25519_spki(&peer_leaf).unwrap();
        assert_eq!(extracted, peer_public);

        let server_identity = store.device_tls_identity(&config.device_id).unwrap();
        let ca = store.fabric_tls_ca_der().unwrap();
        build_rustls_server_config(server_identity, vec![ca]).unwrap();
    }

    #[test]
    fn spki_extraction_rejects_missing_and_ambiguous_keys() {
        let missing = extract_single_ed25519_spki(&[0u8; 64]).unwrap_err();
        assert!(missing.to_string().contains("does not contain"));

        let mut ambiguous = spki_from_public_key(&[1u8; 32]);
        ambiguous.extend_from_slice(&spki_from_public_key(&[2u8; 32]));
        let err = extract_single_ed25519_spki(&ambiguous).unwrap_err();
        assert!(err.to_string().contains("more than one"));
    }
}
