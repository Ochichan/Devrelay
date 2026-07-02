//! Remote access credential bundles.
//!
//! After pairing confirms a peer device, the fabric owner issues one portable
//! bundle holding everything the peer needs to call the remote Control RPC
//! API: the fabric X.509 CA, the peer's TLS leaf certificate, the peer's
//! fabric-signed device certificate, and the issuer's signing key for server
//! pinning. The peer transports the bundle over the channel the pairing code
//! already authenticated, validates that it matches its own signing key and
//! fabric root, and stores it inside the identity directory. The peer's
//! private key never leaves the peer.

use crate::{
    DevRelayError, DevRelayHome, DeviceCertificate, DevicePublicIdentity, Result,
    extract_single_ed25519_spki, unix_now_seconds,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub const REMOTE_ACCESS_CREDENTIALS_SCHEMA_VERSION: u32 = 1;
pub const REMOTE_ACCESS_CREDENTIALS_FILE_NAME: &str = "remote-access-credentials.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteAccessCredentialBundle {
    pub schema_version: u32,
    pub fabric_id: String,
    pub issuer_device_id: String,
    pub issuer_signing_public_key_hex: String,
    pub subject_device_id: String,
    pub subject_signing_public_key_hex: String,
    pub fabric_tls_ca_der_hex: String,
    pub subject_tls_leaf_der_hex: String,
    pub device_certificate: DeviceCertificate,
    pub issued_at_unix_seconds: u64,
}

impl RemoteAccessCredentialBundle {
    pub fn fabric_tls_ca_der(&self) -> Result<Vec<u8>> {
        decode_hex("fabric_tls_ca_der_hex", &self.fabric_tls_ca_der_hex)
    }

    pub fn subject_tls_leaf_der(&self) -> Result<Vec<u8>> {
        decode_hex("subject_tls_leaf_der_hex", &self.subject_tls_leaf_der_hex)
    }
}

/// Assembles a credential bundle for a confirmed peer.
///
/// The caller supplies the peer's device certificate from the confirmed
/// pairing session; issuance fails when the certificate does not describe the
/// subject device.
pub fn assemble_remote_access_credentials(
    issuer: &DevicePublicIdentity,
    subject_device_id: &str,
    subject_signing_public_key_hex: &str,
    fabric_tls_ca_der: Vec<u8>,
    subject_tls_leaf_der: Vec<u8>,
    device_certificate: DeviceCertificate,
) -> Result<RemoteAccessCredentialBundle> {
    if device_certificate.device_id != subject_device_id {
        return Err(DevRelayError::Config(format!(
            "device certificate belongs to {}, not subject {subject_device_id}",
            device_certificate.device_id
        )));
    }
    if device_certificate.signing_public_key_hex != subject_signing_public_key_hex {
        return Err(DevRelayError::Config(
            "device certificate signing key does not match the subject signing key".to_string(),
        ));
    }
    if device_certificate.fabric_id != issuer.fabric_id {
        return Err(DevRelayError::Config(
            "device certificate fabric does not match the issuing fabric".to_string(),
        ));
    }
    let leaf_key = extract_single_ed25519_spki(&subject_tls_leaf_der)?;
    if hex_encode(&leaf_key) != subject_signing_public_key_hex {
        return Err(DevRelayError::Config(
            "subject TLS leaf does not carry the subject signing key".to_string(),
        ));
    }
    Ok(RemoteAccessCredentialBundle {
        schema_version: REMOTE_ACCESS_CREDENTIALS_SCHEMA_VERSION,
        fabric_id: issuer.fabric_id.clone(),
        issuer_device_id: issuer.device_id.clone(),
        issuer_signing_public_key_hex: issuer.signing_public_key_hex.clone(),
        subject_device_id: subject_device_id.to_string(),
        subject_signing_public_key_hex: subject_signing_public_key_hex.to_string(),
        fabric_tls_ca_der_hex: hex_encode(&fabric_tls_ca_der),
        subject_tls_leaf_der_hex: hex_encode(&subject_tls_leaf_der),
        device_certificate,
        issued_at_unix_seconds: unix_now_seconds(),
    })
}

/// Validates an imported bundle against the importing device's identity.
pub fn validate_remote_access_credentials(
    bundle: &RemoteAccessCredentialBundle,
    own_device: &DevicePublicIdentity,
) -> Result<()> {
    if bundle.schema_version != REMOTE_ACCESS_CREDENTIALS_SCHEMA_VERSION {
        return Err(DevRelayError::Config(format!(
            "unsupported remote credential schema {}, expected {}",
            bundle.schema_version, REMOTE_ACCESS_CREDENTIALS_SCHEMA_VERSION
        )));
    }
    if bundle.subject_signing_public_key_hex != own_device.signing_public_key_hex {
        return Err(DevRelayError::Config(
            "remote credentials were issued for a different device signing key".to_string(),
        ));
    }
    if bundle.device_certificate.device_id != bundle.subject_device_id {
        return Err(DevRelayError::Config(
            "remote credential device certificate does not describe the subject".to_string(),
        ));
    }
    if bundle.device_certificate.signing_public_key_hex != bundle.subject_signing_public_key_hex {
        return Err(DevRelayError::Config(
            "remote credential device certificate carries a different signing key".to_string(),
        ));
    }
    let expected_fabric_id = crate::identity::fabric_id_for_root_public_key_hex(
        &bundle.device_certificate.issuer_root_public_key_hex,
    )?;
    if bundle.fabric_id != expected_fabric_id
        || bundle.device_certificate.fabric_id != expected_fabric_id
    {
        return Err(DevRelayError::Config(
            "remote credential fabric id does not match the issuing fabric root".to_string(),
        ));
    }
    let leaf_key = extract_single_ed25519_spki(&bundle.subject_tls_leaf_der()?)?;
    if hex_encode(&leaf_key) != bundle.subject_signing_public_key_hex {
        return Err(DevRelayError::Config(
            "remote credential TLS leaf does not carry the subject signing key".to_string(),
        ));
    }
    bundle.fabric_tls_ca_der()?;
    Ok(())
}

pub fn remote_access_credentials_path(home: &DevRelayHome) -> std::path::PathBuf {
    home.identity_dir()
        .join(REMOTE_ACCESS_CREDENTIALS_FILE_NAME)
}

pub fn save_remote_access_credentials(
    home: &DevRelayHome,
    bundle: &RemoteAccessCredentialBundle,
) -> Result<()> {
    fs::create_dir_all(home.identity_dir())?;
    let path = remote_access_credentials_path(home);
    let raw = serde_json::to_vec_pretty(bundle)?;
    fs::write(&path, raw)?;
    set_private_file_permissions(&path)?;
    Ok(())
}

pub fn load_remote_access_credentials(home: &DevRelayHome) -> Result<RemoteAccessCredentialBundle> {
    let path = remote_access_credentials_path(home);
    let raw = fs::read_to_string(&path).map_err(|err| {
        DevRelayError::Config(format!(
            "no remote access credentials at {}: {err}; import a bundle first",
            path.display()
        ))
    })?;
    Ok(serde_json::from_str(&raw)?)
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn decode_hex(field: &str, value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err(DevRelayError::Config(format!(
            "{field} must contain an even number of hex characters"
        )));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for chunk in value.as_bytes().chunks_exact(2) {
        let high = hex_value(chunk[0])
            .ok_or_else(|| DevRelayError::Config(format!("{field} contains non-hex characters")))?;
        let low = hex_value(chunk[1])
            .ok_or_else(|| DevRelayError::Config(format!("{field} contains non-hex characters")))?;
        bytes.push((high << 4) | low);
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

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FabricIdentityStore, LocalConfig};

    struct Fabric {
        _temp: tempfile::TempDir,
        home: DevRelayHome,
        store: FabricIdentityStore,
        config: LocalConfig,
    }

    fn fabric() -> Fabric {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path());
        let config = LocalConfig::new_for_local_device();
        let store = FabricIdentityStore::new(home.clone());
        store.open_or_create(&config).unwrap();
        Fabric {
            _temp: temp,
            home,
            store,
            config,
        }
    }

    fn issue_for_peer(owner: &Fabric, peer: &Fabric) -> RemoteAccessCredentialBundle {
        let owner_bundle = owner.store.public_bundle_from_store(&owner.config).unwrap();
        let peer_bundle = peer.store.public_bundle_from_store(&peer.config).unwrap();
        let mut peer_identity = peer_bundle.device.clone();
        peer_identity.fabric_id = owner_bundle.root.fabric_id.clone();
        let certificate = owner
            .store
            .issue_device_certificate(&peer_identity, unix_now_seconds() - 60, 3_600)
            .unwrap();
        let leaf = owner
            .store
            .issue_peer_tls_certificate_der(
                &peer_identity.device_id,
                &peer_identity.signing_public_key_hex,
            )
            .unwrap();
        assemble_remote_access_credentials(
            &owner_bundle.device,
            &peer_identity.device_id,
            &peer_identity.signing_public_key_hex,
            owner.store.fabric_tls_ca_der().unwrap(),
            leaf,
            certificate,
        )
        .unwrap()
    }

    #[test]
    fn issued_bundle_validates_and_round_trips_through_storage() {
        let owner = fabric();
        let peer = fabric();
        let bundle = issue_for_peer(&owner, &peer);
        let peer_device = peer
            .store
            .public_bundle_from_store(&peer.config)
            .unwrap()
            .device;

        validate_remote_access_credentials(&bundle, &peer_device).unwrap();
        save_remote_access_credentials(&peer.home, &bundle).unwrap();
        let loaded = load_remote_access_credentials(&peer.home).unwrap();

        assert_eq!(loaded, bundle);
        assert_eq!(loaded.subject_device_id, peer_device.device_id);
        assert!(!loaded.fabric_tls_ca_der().unwrap().is_empty());
    }

    #[test]
    fn validation_rejects_bundle_for_another_device_key() {
        let owner = fabric();
        let peer = fabric();
        let interloper = fabric();
        let bundle = issue_for_peer(&owner, &peer);
        let interloper_device = interloper
            .store
            .public_bundle_from_store(&interloper.config)
            .unwrap()
            .device;

        let err = validate_remote_access_credentials(&bundle, &interloper_device).unwrap_err();

        assert!(err.to_string().contains("different device signing key"));
    }

    #[test]
    fn validation_rejects_fabric_id_mismatch() {
        let owner = fabric();
        let peer = fabric();
        let mut bundle = issue_for_peer(&owner, &peer);
        bundle.fabric_id = "f_forged000000000000000000".to_string();
        let peer_device = peer
            .store
            .public_bundle_from_store(&peer.config)
            .unwrap()
            .device;

        let err = validate_remote_access_credentials(&bundle, &peer_device).unwrap_err();

        assert!(err.to_string().contains("fabric id does not match"));
    }

    #[test]
    fn assemble_rejects_certificate_for_other_subject() {
        let owner = fabric();
        let peer = fabric();
        let owner_bundle = owner.store.public_bundle_from_store(&owner.config).unwrap();
        let peer_bundle = peer.store.public_bundle_from_store(&peer.config).unwrap();
        let mut peer_identity = peer_bundle.device.clone();
        peer_identity.fabric_id = owner_bundle.root.fabric_id.clone();
        let certificate = owner
            .store
            .issue_device_certificate(&peer_identity, unix_now_seconds(), 3_600)
            .unwrap();
        let leaf = owner
            .store
            .issue_peer_tls_certificate_der(
                &peer_identity.device_id,
                &peer_identity.signing_public_key_hex,
            )
            .unwrap();

        let err = assemble_remote_access_credentials(
            &owner_bundle.device,
            "someone-else",
            &peer_identity.signing_public_key_hex,
            owner.store.fabric_tls_ca_der().unwrap(),
            leaf,
            certificate,
        )
        .unwrap_err();

        assert!(err.to_string().contains("not subject someone-else"));
    }
}
