//! Pairing session primitives.
//!
//! M4.2 records the cryptographic transcript and user-confirmed short
//! authentication string before any later transport is allowed to trust a peer.

use crate::{DevRelayError, Result};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

pub const PAIRING_ID_PREFIX: &str = "pa_";
const KEY_BYTES: usize = 32;
const KEY_HEX_BYTES: usize = KEY_BYTES * 2;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PairingState {
    Pending,
    Confirmed,
    Aborted,
    Expired,
}

impl PairingState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Confirmed => "confirmed",
            Self::Aborted => "aborted",
            Self::Expired => "expired",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "confirmed" => Self::Confirmed,
            "aborted" => Self::Aborted,
            "expired" => Self::Expired,
            _ => Self::Pending,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Confirmed | Self::Aborted | Self::Expired)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairingEphemeralKey {
    pub secret_key_hex: String,
    pub public_key_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairingSession {
    pub pairing_id: String,
    pub fabric_id: String,
    pub local_device_id: String,
    pub peer_device_id: String,
    pub peer_display_name: String,
    pub peer_signing_public_key_hex: String,
    pub peer_network_public_key_hex: String,
    pub anchor_address: Option<String>,
    pub local_ephemeral_public_key_hex: String,
    pub peer_ephemeral_public_key_hex: String,
    pub transcript_hash_hex: String,
    pub short_authentication_string: String,
    pub state: PairingState,
    pub certificate_json: Option<String>,
    pub expires_at_unix_seconds: u64,
    pub confirmed_at_unix_seconds: Option<u64>,
    pub aborted_at_unix_seconds: Option<u64>,
    pub created_at_unix_seconds: u64,
}

pub fn generate_pairing_id() -> String {
    let seed = format!("{}\0{}", std::process::id(), unix_now_nanos());
    let digest = blake3::hash(seed.as_bytes());
    format!("{PAIRING_ID_PREFIX}{}", &digest.to_hex()[..24])
}

pub fn generate_ephemeral_pairing_key() -> Result<PairingEphemeralKey> {
    let mut secret_bytes = [0u8; KEY_BYTES];
    getrandom::getrandom(&mut secret_bytes)
        .map_err(|err| DevRelayError::Config(format!("failed to read OS entropy: {err}")))?;
    let secret = StaticSecret::from(secret_bytes);
    let public = X25519PublicKey::from(&secret).to_bytes();
    Ok(PairingEphemeralKey {
        secret_key_hex: hex_encode(&secret.to_bytes()),
        public_key_hex: hex_encode(&public),
    })
}

pub fn compute_handshake_transcript_hash(
    fabric_id: &str,
    local_device_id: &str,
    peer_device_id: &str,
    local_ephemeral_public_key_hex: &str,
    peer_ephemeral_public_key_hex: &str,
    anchor_address: Option<&str>,
) -> Result<String> {
    validate_key_hex(
        "local_ephemeral_public_key_hex",
        local_ephemeral_public_key_hex,
    )?;
    validate_key_hex(
        "peer_ephemeral_public_key_hex",
        peer_ephemeral_public_key_hex,
    )?;
    let transcript = serde_json::json!({
        "schema": 1,
        "fabric_id": fabric_id,
        "local_device_id": local_device_id,
        "peer_device_id": peer_device_id,
        "local_ephemeral_public_key_hex": local_ephemeral_public_key_hex,
        "peer_ephemeral_public_key_hex": peer_ephemeral_public_key_hex,
        "anchor_address": anchor_address,
    });
    let encoded = serde_json::to_vec(&transcript)?;
    Ok(blake3::hash(&encoded).to_hex().to_string())
}

pub fn derive_short_authentication_string(transcript_hash_hex: &str) -> Result<String> {
    if transcript_hash_hex.len() < 8
        || !transcript_hash_hex
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DevRelayError::Config(
            "transcript_hash_hex must be hex".to_string(),
        ));
    }
    let value = u32::from_str_radix(&transcript_hash_hex[..8], 16)
        .map_err(|err| DevRelayError::Config(format!("invalid transcript hash: {err}")))?;
    Ok(format!("{:06}", value % 1_000_000))
}

pub fn validate_key_hex(field: &str, value: &str) -> Result<()> {
    if value.len() != KEY_HEX_BYTES || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DevRelayError::Config(format!(
            "{field} must be {KEY_HEX_BYTES} hex characters"
        )));
    }
    Ok(())
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

fn unix_now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_stable_short_authentication_string() {
        let local = "a".repeat(64);
        let peer = "b".repeat(64);
        let hash = compute_handshake_transcript_hash(
            "f_123",
            "d_local",
            "d_peer",
            &local,
            &peer,
            Some("192.0.2.1:7000"),
        )
        .unwrap();
        assert_eq!(hash.len(), 64);
        let code = derive_short_authentication_string(&hash).unwrap();
        assert_eq!(code.len(), 6);
        assert!(code.bytes().all(|byte| byte.is_ascii_digit()));
        assert_eq!(code, derive_short_authentication_string(&hash).unwrap());
    }

    #[test]
    fn generates_ephemeral_pairing_key() {
        let key = generate_ephemeral_pairing_key().unwrap();
        assert_eq!(key.secret_key_hex.len(), 64);
        assert_eq!(key.public_key_hex.len(), 64);
        assert_ne!(key.secret_key_hex, key.public_key_hex);
    }
}
