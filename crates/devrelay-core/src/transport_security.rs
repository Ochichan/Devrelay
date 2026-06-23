//! Secure control-plane transport configuration and peer certificate policy.

use crate::{DevRelayError, DeviceCertificate, DeviceRevocationRecord, FabricRootIdentity, Result};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rustls::RootCertStore;
use rustls::client::ClientConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ServerConfig, WebPkiClientVerifier};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

pub const CONTROL_PROTOCOL_VERSION: u32 = 1;
pub const CONTROL_ALPN_PROTOCOL: &[u8] = b"devrelay-control/1";
pub const DEFAULT_MAX_CLOCK_SKEW_SECONDS: u64 = 300;
pub const DEFAULT_REPLAY_WINDOW_SECONDS: u64 = 300;
pub const DEFAULT_CONNECTION_TIMEOUT_MILLIS: u64 = 10_000;
pub const DEFAULT_REQUEST_TIMEOUT_MILLIS: u64 = 30_000;

const MIN_REPLAY_NONCE_LEN: usize = 16;
const MAX_REPLAY_NONCE_LEN: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustlsIdentity {
    pub cert_chain_der: Vec<Vec<u8>>,
    pub private_key_pkcs8_der: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidatedDeviceCertificate {
    pub certificate_id: String,
    pub fabric_id: String,
    pub device_id: String,
    pub signing_public_key_hex: String,
    pub network_public_key_hex: String,
    pub expires_at_unix_seconds: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ControlPlaneTransportSecurity {
    Mtls,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthenticatedControlPlanePeer {
    pub transport: ControlPlaneTransportSecurity,
    pub device: ValidatedDeviceCertificate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneTransportPolicy {
    pub protocol_version: u32,
    pub max_clock_skew_seconds: u64,
    pub replay_window_seconds: u64,
    pub connection_timeout_millis: u64,
    pub request_timeout_millis: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneRequestEnvelope {
    pub protocol_version: u32,
    pub sent_at_unix_seconds: u64,
    pub replay_nonce: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ControlPlaneReplayCache {
    seen: BTreeMap<String, u64>,
}

impl Default for ControlPlaneTransportPolicy {
    fn default() -> Self {
        Self {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            max_clock_skew_seconds: DEFAULT_MAX_CLOCK_SKEW_SECONDS,
            replay_window_seconds: DEFAULT_REPLAY_WINDOW_SECONDS,
            connection_timeout_millis: DEFAULT_CONNECTION_TIMEOUT_MILLIS,
            request_timeout_millis: DEFAULT_REQUEST_TIMEOUT_MILLIS,
        }
    }
}

impl ControlPlaneTransportPolicy {
    pub fn validate(&self) -> Result<()> {
        if self.protocol_version == 0 {
            return Err(DevRelayError::Config(
                "control protocol version must be non-zero".to_string(),
            ));
        }
        if self.max_clock_skew_seconds == 0 {
            return Err(DevRelayError::Config(
                "max clock skew must be non-zero".to_string(),
            ));
        }
        if self.replay_window_seconds == 0 {
            return Err(DevRelayError::Config(
                "replay window must be non-zero".to_string(),
            ));
        }
        if self.connection_timeout_millis == 0 {
            return Err(DevRelayError::Config(
                "connection timeout must be non-zero".to_string(),
            ));
        }
        if self.request_timeout_millis == 0 {
            return Err(DevRelayError::Config(
                "request timeout must be non-zero".to_string(),
            ));
        }
        Ok(())
    }
}

impl ControlPlaneReplayCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn remember(
        &mut self,
        nonce: &str,
        now_unix_seconds: u64,
        replay_window_seconds: u64,
    ) -> Result<()> {
        self.prune(now_unix_seconds, replay_window_seconds);
        if self.seen.contains_key(nonce) {
            return Err(DevRelayError::Config(format!(
                "control request replay nonce {nonce} was already used"
            )));
        }
        self.seen.insert(nonce.to_string(), now_unix_seconds);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    fn prune(&mut self, now_unix_seconds: u64, replay_window_seconds: u64) {
        self.seen.retain(|_, seen_at| {
            *seen_at > now_unix_seconds
                || now_unix_seconds.saturating_sub(*seen_at) <= replay_window_seconds
        });
    }
}

pub fn negotiate_control_protocol_version(
    client_supported_versions: &[u32],
    policy: &ControlPlaneTransportPolicy,
) -> Result<u32> {
    policy.validate()?;
    if client_supported_versions.contains(&policy.protocol_version) {
        return Ok(policy.protocol_version);
    }
    Err(DevRelayError::Config(format!(
        "control protocol version mismatch: server requires {}, client offered {:?}",
        policy.protocol_version, client_supported_versions
    )))
}

pub fn validate_control_request_envelope(
    policy: &ControlPlaneTransportPolicy,
    envelope: &ControlPlaneRequestEnvelope,
    now_unix_seconds: u64,
    replay_cache: &mut ControlPlaneReplayCache,
) -> Result<()> {
    policy.validate()?;
    if envelope.protocol_version != policy.protocol_version {
        return Err(DevRelayError::Config(format!(
            "control request protocol version {} does not match expected {}",
            envelope.protocol_version, policy.protocol_version
        )));
    }
    validate_replay_nonce(&envelope.replay_nonce)?;
    let clock_delta = now_unix_seconds.abs_diff(envelope.sent_at_unix_seconds);
    if clock_delta > policy.max_clock_skew_seconds {
        return Err(DevRelayError::Config(format!(
            "control request timestamp skew {clock_delta}s exceeds max {}s",
            policy.max_clock_skew_seconds
        )));
    }
    replay_cache.remember(
        &envelope.replay_nonce,
        now_unix_seconds,
        policy.replay_window_seconds,
    )
}

pub fn require_authenticated_control_channel(
    peer: Option<ValidatedDeviceCertificate>,
) -> Result<AuthenticatedControlPlanePeer> {
    let device = peer.ok_or_else(|| {
        DevRelayError::Config(
            "control API request rejected: authenticated mTLS peer is required".to_string(),
        )
    })?;
    Ok(AuthenticatedControlPlanePeer {
        transport: ControlPlaneTransportSecurity::Mtls,
        device,
    })
}

pub fn build_rustls_server_config(
    identity: RustlsIdentity,
    trusted_client_roots_der: Vec<Vec<u8>>,
) -> Result<Arc<ServerConfig>> {
    validate_rustls_identity(&identity)?;
    let RustlsIdentity {
        cert_chain_der,
        private_key_pkcs8_der,
    } = identity;
    if trusted_client_roots_der.is_empty() {
        return Err(DevRelayError::Config(
            "rustls server config requires at least one trusted client root".to_string(),
        ));
    }
    let client_roots = root_store_from_der(trusted_client_roots_der, "trusted client root")?;
    let client_verifier = WebPkiClientVerifier::builder(Arc::new(client_roots))
        .build()
        .map_err(|err| DevRelayError::Config(format!("invalid client verifier: {err}")))?;
    let mut config = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(
            cert_chain_from_der(cert_chain_der),
            private_key(private_key_pkcs8_der),
        )
        .map_err(|err| DevRelayError::Config(format!("invalid rustls server identity: {err}")))?;
    config.alpn_protocols = vec![CONTROL_ALPN_PROTOCOL.to_vec()];
    Ok(Arc::new(config))
}

pub fn build_rustls_client_config(
    identity: RustlsIdentity,
    trusted_server_roots_der: Vec<Vec<u8>>,
) -> Result<Arc<ClientConfig>> {
    validate_rustls_identity(&identity)?;
    let RustlsIdentity {
        cert_chain_der,
        private_key_pkcs8_der,
    } = identity;
    if trusted_server_roots_der.is_empty() {
        return Err(DevRelayError::Config(
            "rustls client config requires at least one trusted server root".to_string(),
        ));
    }
    let server_roots = root_store_from_der(trusted_server_roots_der, "trusted server root")?;
    let mut config = ClientConfig::builder()
        .with_root_certificates(server_roots)
        .with_client_auth_cert(
            cert_chain_from_der(cert_chain_der),
            private_key(private_key_pkcs8_der),
        )
        .map_err(|err| DevRelayError::Config(format!("invalid rustls client identity: {err}")))?;
    config.alpn_protocols = vec![CONTROL_ALPN_PROTOCOL.to_vec()];
    Ok(Arc::new(config))
}

pub fn validate_device_certificate(
    certificate: &DeviceCertificate,
    pinned_root: &FabricRootIdentity,
    revocations: &[DeviceRevocationRecord],
    now_unix_seconds: u64,
) -> Result<ValidatedDeviceCertificate> {
    if certificate.fabric_id != pinned_root.fabric_id {
        return Err(DevRelayError::Config(format!(
            "device certificate fabric {} does not match pinned fabric {}",
            certificate.fabric_id, pinned_root.fabric_id
        )));
    }
    if certificate.issuer_root_public_key_hex != pinned_root.root_public_key_hex {
        return Err(DevRelayError::Config(
            "device certificate issuer does not match pinned fabric root".to_string(),
        ));
    }
    if now_unix_seconds < certificate.issued_at_unix_seconds {
        return Err(DevRelayError::Config(format!(
            "device certificate {} is not valid until {}",
            certificate.certificate_id, certificate.issued_at_unix_seconds
        )));
    }
    if now_unix_seconds >= certificate.expires_at_unix_seconds {
        return Err(DevRelayError::Config(format!(
            "device certificate {} expired at {}",
            certificate.certificate_id, certificate.expires_at_unix_seconds
        )));
    }
    if revocations
        .iter()
        .any(|revocation| revocation.device_id == certificate.device_id)
    {
        return Err(DevRelayError::Config(format!(
            "device certificate {} rejected: device {} is revoked",
            certificate.certificate_id, certificate.device_id
        )));
    }

    decode_hex_array::<32>(
        "signing_public_key_hex",
        &certificate.signing_public_key_hex,
    )?;
    decode_hex_array::<32>(
        "network_public_key_hex",
        &certificate.network_public_key_hex,
    )?;
    let root_public_key =
        decode_hex_array::<32>("root_public_key_hex", &pinned_root.root_public_key_hex)?;
    let signature = decode_hex_array::<64>("signature_hex", &certificate.signature_hex)?;
    let payload_bytes = signed_certificate_payload(certificate)?;
    let expected_certificate_id = format!("cert_{}", &blake3::hash(&payload_bytes).to_hex()[..24]);
    if certificate.certificate_id != expected_certificate_id {
        return Err(DevRelayError::Config(format!(
            "device certificate id {} does not match signed payload",
            certificate.certificate_id
        )));
    }
    let root_key = VerifyingKey::from_bytes(&root_public_key)
        .map_err(|err| DevRelayError::Config(format!("invalid fabric root key: {err}")))?;
    root_key
        .verify(&payload_bytes, &Signature::from_bytes(&signature))
        .map_err(|err| {
            DevRelayError::Config(format!(
                "device certificate {} signature is invalid: {err}",
                certificate.certificate_id
            ))
        })?;

    Ok(ValidatedDeviceCertificate {
        certificate_id: certificate.certificate_id.clone(),
        fabric_id: certificate.fabric_id.clone(),
        device_id: certificate.device_id.clone(),
        signing_public_key_hex: certificate.signing_public_key_hex.clone(),
        network_public_key_hex: certificate.network_public_key_hex.clone(),
        expires_at_unix_seconds: certificate.expires_at_unix_seconds,
    })
}

fn validate_rustls_identity(identity: &RustlsIdentity) -> Result<()> {
    if identity.cert_chain_der.is_empty() {
        return Err(DevRelayError::Config(
            "rustls identity requires a certificate chain".to_string(),
        ));
    }
    if identity.private_key_pkcs8_der.is_empty() {
        return Err(DevRelayError::Config(
            "rustls identity requires a PKCS#8 private key".to_string(),
        ));
    }
    Ok(())
}

fn root_store_from_der(certs: Vec<Vec<u8>>, label: &str) -> Result<RootCertStore> {
    let mut roots = RootCertStore::empty();
    for cert in certs {
        roots.add(CertificateDer::from(cert)).map_err(|err| {
            DevRelayError::Config(format!("invalid {label} certificate DER: {err}"))
        })?;
    }
    Ok(roots)
}

fn cert_chain_from_der(certs: Vec<Vec<u8>>) -> Vec<CertificateDer<'static>> {
    certs.into_iter().map(CertificateDer::from).collect()
}

fn private_key(private_key_pkcs8_der: Vec<u8>) -> PrivateKeyDer<'static> {
    PrivatePkcs8KeyDer::from(private_key_pkcs8_der).into()
}

fn signed_certificate_payload(certificate: &DeviceCertificate) -> Result<Vec<u8>> {
    let payload = serde_json::json!({
        "schema": 1,
        "fabric_id": certificate.fabric_id,
        "device_id": certificate.device_id,
        "signing_public_key_hex": certificate.signing_public_key_hex,
        "network_public_key_hex": certificate.network_public_key_hex,
        "issuer_root_public_key_hex": certificate.issuer_root_public_key_hex,
        "issued_at_unix_seconds": certificate.issued_at_unix_seconds,
        "expires_at_unix_seconds": certificate.expires_at_unix_seconds,
    });
    serde_json::to_vec(&payload).map_err(Into::into)
}

fn validate_replay_nonce(nonce: &str) -> Result<()> {
    if nonce.len() < MIN_REPLAY_NONCE_LEN || nonce.len() > MAX_REPLAY_NONCE_LEN {
        return Err(DevRelayError::Config(format!(
            "control request replay nonce must be between {MIN_REPLAY_NONCE_LEN} and {MAX_REPLAY_NONCE_LEN} characters"
        )));
    }
    if !nonce
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(DevRelayError::Config(
            "control request replay nonce must be base64url-safe".to_string(),
        ));
    }
    Ok(())
}

fn decode_hex_array<const N: usize>(field: &str, value: &str) -> Result<[u8; N]> {
    let expected_len = N * 2;
    if value.len() != expected_len {
        return Err(DevRelayError::Config(format!(
            "{field} must be {expected_len} hex characters"
        )));
    }
    let mut bytes = [0u8; N];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_value(chunk[0])
            .ok_or_else(|| DevRelayError::Config(format!("{field} contains non-hex characters")))?;
        let low = hex_value(chunk[1])
            .ok_or_else(|| DevRelayError::Config(format!("{field} contains non-hex characters")))?;
        bytes[index] = (high << 4) | low;
    }
    Ok(bytes)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DevRelayHome, FabricIdentityStore, LocalConfig};

    #[test]
    fn builds_rustls_server_and_client_configs_with_mtls_roots() {
        let identity = test_rustls_identity();
        let roots = identity.cert_chain_der.clone();

        let server =
            build_rustls_server_config(identity.clone(), roots.clone()).expect("server config");
        let client = build_rustls_client_config(identity, roots).expect("client config");

        assert_eq!(server.alpn_protocols, vec![CONTROL_ALPN_PROTOCOL.to_vec()]);
        assert_eq!(client.alpn_protocols, vec![CONTROL_ALPN_PROTOCOL.to_vec()]);
    }

    #[test]
    fn rejects_rustls_configs_without_trust_roots() {
        let identity = test_rustls_identity();

        let server = build_rustls_server_config(identity.clone(), Vec::new()).unwrap_err();
        let client = build_rustls_client_config(identity, Vec::new()).unwrap_err();

        assert!(server.to_string().contains("trusted client root"));
        assert!(client.to_string().contains("trusted server root"));
    }

    #[test]
    fn negotiates_control_protocol_version() {
        let policy = ControlPlaneTransportPolicy::default();

        let negotiated =
            negotiate_control_protocol_version(&[0, CONTROL_PROTOCOL_VERSION], &policy)
                .expect("protocol negotiation");
        let err = negotiate_control_protocol_version(&[0, 99], &policy).unwrap_err();

        assert_eq!(negotiated, CONTROL_PROTOCOL_VERSION);
        assert!(err.to_string().contains("version mismatch"));
    }

    #[test]
    fn validates_control_request_timestamp_and_replay_nonce() {
        let policy = ControlPlaneTransportPolicy::default();
        let envelope = ControlPlaneRequestEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            sent_at_unix_seconds: 1_000,
            replay_nonce: "nonce_1234567890".to_string(),
        };
        let mut cache = ControlPlaneReplayCache::new();

        validate_control_request_envelope(&policy, &envelope, 1_001, &mut cache)
            .expect("valid request envelope");
        let replay =
            validate_control_request_envelope(&policy, &envelope, 1_002, &mut cache).unwrap_err();

        assert_eq!(cache.len(), 1);
        assert!(replay.to_string().contains("already used"));
    }

    #[test]
    fn rejects_control_request_clock_skew_and_protocol_mismatch() {
        let policy = ControlPlaneTransportPolicy::default();
        let stale = ControlPlaneRequestEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            sent_at_unix_seconds: 1_000,
            replay_nonce: "nonce_stale_1234".to_string(),
        };
        let future = ControlPlaneRequestEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            sent_at_unix_seconds: 1_701,
            replay_nonce: "nonce_future_123".to_string(),
        };
        let wrong_version = ControlPlaneRequestEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION + 1,
            sent_at_unix_seconds: 1_000,
            replay_nonce: "nonce_version_12".to_string(),
        };

        let stale_err = validate_control_request_envelope(
            &policy,
            &stale,
            1_301 + policy.max_clock_skew_seconds,
            &mut ControlPlaneReplayCache::new(),
        )
        .unwrap_err();
        let future_err = validate_control_request_envelope(
            &policy,
            &future,
            1_400,
            &mut ControlPlaneReplayCache::new(),
        )
        .unwrap_err();
        let version_err = validate_control_request_envelope(
            &policy,
            &wrong_version,
            1_000,
            &mut ControlPlaneReplayCache::new(),
        )
        .unwrap_err();

        assert!(stale_err.to_string().contains("timestamp skew"));
        assert!(future_err.to_string().contains("timestamp skew"));
        assert!(version_err.to_string().contains("protocol version"));
    }

    #[test]
    fn rejects_short_or_unsafe_replay_nonce() {
        let policy = ControlPlaneTransportPolicy::default();
        let short = ControlPlaneRequestEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            sent_at_unix_seconds: 1_000,
            replay_nonce: "short".to_string(),
        };
        let unsafe_nonce = ControlPlaneRequestEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            sent_at_unix_seconds: 1_000,
            replay_nonce: "nonce with spaces".to_string(),
        };

        let short_err = validate_control_request_envelope(
            &policy,
            &short,
            1_000,
            &mut ControlPlaneReplayCache::new(),
        )
        .unwrap_err();
        let unsafe_err = validate_control_request_envelope(
            &policy,
            &unsafe_nonce,
            1_000,
            &mut ControlPlaneReplayCache::new(),
        )
        .unwrap_err();

        assert!(short_err.to_string().contains("between"));
        assert!(unsafe_err.to_string().contains("base64url-safe"));
    }

    #[test]
    fn default_control_transport_policy_sets_bounded_timeouts() {
        let policy = ControlPlaneTransportPolicy::default();

        policy.validate().unwrap();

        assert_eq!(policy.connection_timeout_millis, 10_000);
        assert_eq!(policy.request_timeout_millis, 30_000);
        assert_eq!(policy.max_clock_skew_seconds, 300);
        assert_eq!(policy.replay_window_seconds, 300);
    }

    #[test]
    fn control_channel_requires_authenticated_mtls_peer() {
        let (bundle, certificate) = test_device_certificate();
        let validated = validate_device_certificate(&certificate, &bundle.root, &[], 120).unwrap();

        let peer = require_authenticated_control_channel(Some(validated.clone())).unwrap();

        assert_eq!(peer.transport, ControlPlaneTransportSecurity::Mtls);
        assert_eq!(peer.device, validated);

        let rejected = require_authenticated_control_channel(None).unwrap_err();
        assert!(rejected.to_string().contains("authenticated mTLS peer"));
    }

    #[test]
    fn validates_device_certificate_against_pinned_fabric_root() {
        let (bundle, certificate) = test_device_certificate();

        let validated = validate_device_certificate(&certificate, &bundle.root, &[], 120).unwrap();

        assert_eq!(validated.certificate_id, certificate.certificate_id);
        assert_eq!(validated.device_id, bundle.device.device_id);
        assert_eq!(validated.fabric_id, bundle.root.fabric_id);
    }

    #[test]
    fn rejects_expired_device_certificate() {
        let (bundle, certificate) = test_device_certificate();

        let err = validate_device_certificate(&certificate, &bundle.root, &[], 160).unwrap_err();

        assert!(err.to_string().contains("expired"));
    }

    #[test]
    fn rejects_revoked_device_certificate() {
        let (bundle, certificate) = test_device_certificate();
        let revocation = DeviceRevocationRecord {
            device_id: certificate.device_id.clone(),
            revoked_by_device_id: "security".to_string(),
            reason: "lost laptop".to_string(),
            key_rotation_required: true,
            revoked_at_unix_seconds: 110,
        };

        let err = validate_device_certificate(&certificate, &bundle.root, &[revocation], 120)
            .unwrap_err();

        assert!(err.to_string().contains("revoked"));
    }

    #[test]
    fn rejects_wrong_fabric_device_certificate() {
        let (_bundle, certificate) = test_device_certificate();
        let other_root = test_device_certificate_with_name("Other Fabric").0.root;

        let err = validate_device_certificate(&certificate, &other_root, &[], 120).unwrap_err();

        assert!(err.to_string().contains("pinned fabric"));
    }

    #[test]
    fn rejects_tampered_device_certificate_signature() {
        let (bundle, mut certificate) = test_device_certificate();
        certificate.network_public_key_hex = "a".repeat(64);

        let err = validate_device_certificate(&certificate, &bundle.root, &[], 120).unwrap_err();

        assert!(err.to_string().contains("signed payload"));
    }

    fn test_rustls_identity() -> RustlsIdentity {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        RustlsIdentity {
            cert_chain_der: vec![certified.cert.der().as_ref().to_vec()],
            private_key_pkcs8_der: certified.key_pair.serialize_der(),
        }
    }

    fn test_device_certificate() -> (crate::FabricIdentityBundle, DeviceCertificate) {
        test_device_certificate_with_name("Test Fabric")
    }

    fn test_device_certificate_with_name(
        fabric_name: &str,
    ) -> (crate::FabricIdentityBundle, DeviceCertificate) {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path());
        let mut config = LocalConfig::new_for_local_device();
        config.fabric_name = fabric_name.to_string();
        let store = FabricIdentityStore::new(home);
        let bundle = store.open_or_create(&config).unwrap();
        let certificate = store
            .issue_device_certificate(&bundle.device, 100, 60)
            .unwrap();
        (bundle, certificate)
    }
}
