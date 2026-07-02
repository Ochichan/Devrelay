//! Fabric and device identity primitives.
//!
//! M4 starts with a local dev-mode identity store. Private keys are generated
//! from OS entropy and written outside project repositories under
//! `DEVRELAY_HOME/identity` with owner-only permissions on Unix platforms.

use crate::{DevRelayError, DevRelayHome, LocalConfig, Result, unix_now_seconds};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

pub const FABRIC_ID_PREFIX: &str = "f_";
const FABRIC_SECRET_SCHEMA_VERSION: u32 = 1;
const KEY_BYTES: usize = 32;
const KEY_HEX_BYTES: usize = KEY_BYTES * 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FabricRootIdentity {
    pub fabric_id: String,
    pub fabric_name: String,
    pub root_public_key_hex: String,
    pub created_at_unix_seconds: u64,
    pub rotation_epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DevicePublicIdentity {
    pub device_id: String,
    pub display_name: String,
    pub fabric_id: String,
    pub signing_public_key_hex: String,
    pub network_public_key_hex: String,
    pub platform_key: String,
    pub architecture: String,
    pub created_at_unix_seconds: u64,
    pub last_seen_unix_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceCertificate {
    pub certificate_id: String,
    pub fabric_id: String,
    pub device_id: String,
    pub signing_public_key_hex: String,
    pub network_public_key_hex: String,
    pub issuer_root_public_key_hex: String,
    pub issued_at_unix_seconds: u64,
    pub expires_at_unix_seconds: u64,
    pub signature_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceRevocationRecord {
    pub device_id: String,
    pub revoked_by_device_id: String,
    pub reason: String,
    pub key_rotation_required: bool,
    pub revoked_at_unix_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryExportStatus {
    pub available: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FabricIdentityBundle {
    pub root: FabricRootIdentity,
    pub device: DevicePublicIdentity,
    pub recovery_export: RecoveryExportStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FabricIdentityStore {
    home: DevRelayHome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FabricSecretFile {
    schema_version: u32,
    fabric_id: String,
    root_secret_key_hex: String,
    device_signing_secret_key_hex: String,
    network_secret_key_hex: String,
    created_at_unix_seconds: u64,
    rotation_epoch: u64,
}

#[derive(Debug, Clone)]
struct FabricSecrets {
    fabric_id: String,
    root_secret_key: [u8; KEY_BYTES],
    device_signing_secret_key: [u8; KEY_BYTES],
    network_secret_key: [u8; KEY_BYTES],
    created_at_unix_seconds: u64,
    rotation_epoch: u64,
}

impl FabricIdentityStore {
    pub fn new(home: DevRelayHome) -> Self {
        Self { home }
    }

    pub fn open_or_create(&self, config: &LocalConfig) -> Result<FabricIdentityBundle> {
        let secrets = if self.home.fabric_secret_path().exists() {
            self.load_secrets()?
        } else {
            self.create_secrets()?
        };
        self.public_bundle(config, &secrets)
    }

    pub fn public_bundle_from_store(&self, config: &LocalConfig) -> Result<FabricIdentityBundle> {
        let secrets = self.load_secrets()?;
        self.public_bundle(config, &secrets)
    }

    pub fn issue_device_certificate(
        &self,
        device: &DevicePublicIdentity,
        issued_at_unix_seconds: u64,
        ttl_seconds: u64,
    ) -> Result<DeviceCertificate> {
        let secrets = self.load_secrets()?;
        if device.fabric_id != secrets.fabric_id {
            return Err(DevRelayError::Config(
                "cannot issue certificate for a different fabric".to_string(),
            ));
        }
        let root_key = SigningKey::from_bytes(&secrets.root_secret_key);
        let issuer_root_public_key_hex = hex_encode(&root_key.verifying_key().to_bytes());
        let expires_at_unix_seconds = issued_at_unix_seconds.saturating_add(ttl_seconds.max(1));
        let payload = serde_json::json!({
            "schema": 1,
            "fabric_id": device.fabric_id,
            "device_id": device.device_id,
            "signing_public_key_hex": device.signing_public_key_hex,
            "network_public_key_hex": device.network_public_key_hex,
            "issuer_root_public_key_hex": issuer_root_public_key_hex,
            "issued_at_unix_seconds": issued_at_unix_seconds,
            "expires_at_unix_seconds": expires_at_unix_seconds,
        });
        let payload_bytes = serde_json::to_vec(&payload)?;
        let signature = root_key.sign(&payload_bytes);
        Ok(DeviceCertificate {
            certificate_id: format!("cert_{}", &blake3::hash(&payload_bytes).to_hex()[..24]),
            fabric_id: device.fabric_id.clone(),
            device_id: device.device_id.clone(),
            signing_public_key_hex: device.signing_public_key_hex.clone(),
            network_public_key_hex: device.network_public_key_hex.clone(),
            issuer_root_public_key_hex,
            issued_at_unix_seconds,
            expires_at_unix_seconds,
            signature_hex: hex_encode(&signature.to_bytes()),
        })
    }

    /// Deterministic fabric X.509 CA certificate for the mTLS control plane.
    ///
    /// Every holder of the fabric root secret regenerates identical DER, so
    /// paired dev-mode devices share one trust root without extra exchange.
    pub fn fabric_tls_ca_der(&self) -> Result<Vec<u8>> {
        let secrets = self.load_secrets()?;
        crate::tls_identity::fabric_tls_ca_der(&secrets.root_secret_key, &secrets.fabric_id)
    }

    /// TLS identity (leaf certificate and PKCS#8 key) for this device.
    ///
    /// The leaf carries the device ed25519 signing key so remote peers can
    /// bind the TLS channel to this device's `DeviceCertificate`.
    pub fn device_tls_identity(&self, device_id: &str) -> Result<crate::RustlsIdentity> {
        let secrets = self.load_secrets()?;
        crate::tls_identity::device_tls_identity(
            &secrets.root_secret_key,
            &secrets.fabric_id,
            device_id,
            &secrets.device_signing_secret_key,
        )
    }

    /// Issues a control-plane TLS leaf certificate for a paired peer device.
    ///
    /// Only the peer's public signing key is required; the private key stays
    /// on the peer device.
    pub fn issue_peer_tls_certificate_der(
        &self,
        peer_device_id: &str,
        peer_signing_public_key_hex: &str,
    ) -> Result<Vec<u8>> {
        let secrets = self.load_secrets()?;
        let peer_public_key =
            decode_key_hex("peer_signing_public_key_hex", peer_signing_public_key_hex)?;
        crate::tls_identity::issue_device_tls_leaf_der(
            &secrets.root_secret_key,
            &secrets.fabric_id,
            peer_device_id,
            &peer_public_key,
        )
    }

    fn create_secrets(&self) -> Result<FabricSecrets> {
        fs::create_dir_all(self.home.identity_dir())?;
        set_private_dir_permissions(&self.home.identity_dir())?;

        let root_secret_key = random_key()?;
        let device_signing_secret_key = random_key()?;
        let network_secret_key = random_key()?;
        let root_public_key = SigningKey::from_bytes(&root_secret_key)
            .verifying_key()
            .to_bytes();
        let fabric_id = fabric_id_for_root_public_key(&root_public_key);
        let secrets = FabricSecrets {
            fabric_id,
            root_secret_key,
            device_signing_secret_key,
            network_secret_key,
            created_at_unix_seconds: unix_now_seconds(),
            rotation_epoch: 0,
        };
        write_secret_file(&self.home.fabric_secret_path(), &secrets)?;
        Ok(secrets)
    }

    fn load_secrets(&self) -> Result<FabricSecrets> {
        let raw = fs::read_to_string(self.home.fabric_secret_path())?;
        let file: FabricSecretFile = serde_json::from_str(&raw)?;
        if file.schema_version != FABRIC_SECRET_SCHEMA_VERSION {
            return Err(DevRelayError::Config(format!(
                "unsupported fabric secret schema {}, expected {}",
                file.schema_version, FABRIC_SECRET_SCHEMA_VERSION
            )));
        }
        Ok(FabricSecrets {
            fabric_id: file.fabric_id,
            root_secret_key: decode_key_hex("root_secret_key_hex", &file.root_secret_key_hex)?,
            device_signing_secret_key: decode_key_hex(
                "device_signing_secret_key_hex",
                &file.device_signing_secret_key_hex,
            )?,
            network_secret_key: decode_key_hex(
                "network_secret_key_hex",
                &file.network_secret_key_hex,
            )?,
            created_at_unix_seconds: file.created_at_unix_seconds,
            rotation_epoch: file.rotation_epoch,
        })
    }

    fn public_bundle(
        &self,
        config: &LocalConfig,
        secrets: &FabricSecrets,
    ) -> Result<FabricIdentityBundle> {
        let root_public_key = SigningKey::from_bytes(&secrets.root_secret_key)
            .verifying_key()
            .to_bytes();
        let expected_fabric_id = fabric_id_for_root_public_key(&root_public_key);
        if expected_fabric_id != secrets.fabric_id {
            return Err(DevRelayError::Config(
                "fabric secret root key does not match stored fabric_id".to_string(),
            ));
        }

        let device_public_key = SigningKey::from_bytes(&secrets.device_signing_secret_key)
            .verifying_key()
            .to_bytes();
        let network_secret = StaticSecret::from(secrets.network_secret_key);
        let network_public = X25519PublicKey::from(&network_secret).to_bytes();

        Ok(FabricIdentityBundle {
            root: FabricRootIdentity {
                fabric_id: secrets.fabric_id.clone(),
                fabric_name: config.fabric_name.clone(),
                root_public_key_hex: hex_encode(&root_public_key),
                created_at_unix_seconds: secrets.created_at_unix_seconds,
                rotation_epoch: secrets.rotation_epoch,
            },
            device: DevicePublicIdentity {
                device_id: config.device_id.clone(),
                display_name: config.device_name.clone(),
                fabric_id: secrets.fabric_id.clone(),
                signing_public_key_hex: hex_encode(&device_public_key),
                network_public_key_hex: hex_encode(&network_public),
                platform_key: config.platform_key.clone(),
                architecture: config.architecture.clone(),
                created_at_unix_seconds: secrets.created_at_unix_seconds,
                last_seen_unix_seconds: config.last_seen_unix_seconds,
            },
            recovery_export: RecoveryExportStatus {
                available: false,
                message: "recovery export is reserved for M4 key backup".to_string(),
            },
        })
    }
}

fn write_secret_file(path: &std::path::Path, secrets: &FabricSecrets) -> Result<()> {
    let file = FabricSecretFile {
        schema_version: FABRIC_SECRET_SCHEMA_VERSION,
        fabric_id: secrets.fabric_id.clone(),
        root_secret_key_hex: hex_encode(&secrets.root_secret_key),
        device_signing_secret_key_hex: hex_encode(&secrets.device_signing_secret_key),
        network_secret_key_hex: hex_encode(&secrets.network_secret_key),
        created_at_unix_seconds: secrets.created_at_unix_seconds,
        rotation_epoch: secrets.rotation_epoch,
    };
    let raw = serde_json::to_vec_pretty(&file)?;
    let mut handle = OpenOptions::new().write(true).create_new(true).open(path)?;
    set_private_file_permissions(path)?;
    handle.write_all(&raw)?;
    handle.write_all(b"\n")?;
    handle.sync_all()?;
    set_private_file_permissions(path)?;
    Ok(())
}

fn random_key() -> Result<[u8; KEY_BYTES]> {
    let mut bytes = [0u8; KEY_BYTES];
    getrandom::getrandom(&mut bytes)
        .map_err(|err| DevRelayError::Config(format!("failed to read OS entropy: {err}")))?;
    Ok(bytes)
}

fn fabric_id_for_root_public_key(root_public_key: &[u8; KEY_BYTES]) -> String {
    let digest = blake3::hash(root_public_key);
    format!("{FABRIC_ID_PREFIX}{}", &digest.to_hex()[..24])
}

fn decode_key_hex(field: &str, value: &str) -> Result<[u8; KEY_BYTES]> {
    if value.len() != KEY_HEX_BYTES {
        return Err(DevRelayError::Config(format!(
            "{field} must be {KEY_HEX_BYTES} hex characters"
        )));
    }
    let mut bytes = [0u8; KEY_BYTES];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_value(chunk[0])
            .ok_or_else(|| DevRelayError::Config(format!("{field} contains non-hex characters")))?;
        let low = hex_value(chunk[1])
            .ok_or_else(|| DevRelayError::Config(format!("{field} contains non-hex characters")))?;
        bytes[index] = (high << 4) | low;
    }
    Ok(bytes)
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

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &std::path::Path) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(path: &std::path::Path) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_and_reuses_dev_mode_identity() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path());
        let mut config = LocalConfig::new_for_local_device();
        config.fabric_name = "Test Fabric".to_string();

        let store = FabricIdentityStore::new(home.clone());
        let first = store.open_or_create(&config).unwrap();
        let second = store.open_or_create(&config).unwrap();

        assert_eq!(first, second);
        assert!(first.root.fabric_id.starts_with(FABRIC_ID_PREFIX));
        assert_eq!(first.root.fabric_name, "Test Fabric");
        assert_eq!(first.root.root_public_key_hex.len(), KEY_HEX_BYTES);
        assert_eq!(first.device.device_id, config.device_id);
        assert_eq!(first.device.signing_public_key_hex.len(), KEY_HEX_BYTES);
        assert_eq!(first.device.network_public_key_hex.len(), KEY_HEX_BYTES);
        assert!(home.fabric_secret_path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn stores_dev_mode_secret_with_owner_only_permissions() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path());
        let config = LocalConfig::new_for_local_device();

        FabricIdentityStore::new(home.clone())
            .open_or_create(&config)
            .unwrap();

        let dir_mode = fs::metadata(home.identity_dir())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let file_mode = fs::metadata(home.fabric_secret_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(file_mode, 0o600);
    }

    #[test]
    fn public_identity_serialization_excludes_secret_material() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path());
        let config = LocalConfig::new_for_local_device();
        let bundle = FabricIdentityStore::new(home)
            .open_or_create(&config)
            .unwrap();

        let encoded = serde_json::to_string(&bundle).unwrap();
        assert!(!encoded.contains("secret_key"));
        assert!(!encoded.contains("private"));
        assert!(encoded.contains("root_public_key_hex"));
        assert!(encoded.contains("network_public_key_hex"));
    }

    #[test]
    fn issues_device_certificate_for_same_fabric() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path());
        let config = LocalConfig::new_for_local_device();
        let store = FabricIdentityStore::new(home);
        let bundle = store.open_or_create(&config).unwrap();

        let cert = store
            .issue_device_certificate(&bundle.device, 100, 60)
            .unwrap();
        assert!(cert.certificate_id.starts_with("cert_"));
        assert_eq!(cert.fabric_id, bundle.root.fabric_id);
        assert_eq!(cert.device_id, bundle.device.device_id);
        assert_eq!(
            cert.issuer_root_public_key_hex,
            bundle.root.root_public_key_hex
        );
        assert_eq!(cert.expires_at_unix_seconds, 160);
        assert_eq!(cert.signature_hex.len(), 128);

        let mut wrong_fabric = bundle.device.clone();
        wrong_fabric.fabric_id = "f_other".to_string();
        let err = store
            .issue_device_certificate(&wrong_fabric, 100, 60)
            .unwrap_err();
        assert!(err.to_string().contains("different fabric"));
    }
}
