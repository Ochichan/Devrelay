//! Local mDNS discovery descriptors and daemon wrapper.
//!
//! Discovery advertisements are intentionally project-agnostic. The TXT record
//! only carries protocol, fabric hint, public device ID, and port.

use crate::{DevRelayError, Result};
use mdns_sd::{Receiver, ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DEVRELAY_ANCHOR_SERVICE_TYPE: &str = "_devrelay-anchor._tcp.local.";
pub const DEVRELAY_PEER_SERVICE_TYPE: &str = "_devrelay-peer._tcp.local.";
pub const DEVRELAY_DISCOVERY_PROTOCOL: &str = "1";
pub const DISCOVERY_TXT_PROTOCOL: &str = "protocol";
pub const DISCOVERY_TXT_FABRIC_HINT: &str = "fabric";
pub const DISCOVERY_TXT_DEVICE_ID: &str = "device_id";
pub const DISCOVERY_TXT_PORT: &str = "port";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DiscoveryRole {
    Anchor,
    Peer,
}

impl DiscoveryRole {
    pub const fn service_type(self) -> &'static str {
        match self {
            Self::Anchor => DEVRELAY_ANCHOR_SERVICE_TYPE,
            Self::Peer => DEVRELAY_PEER_SERVICE_TYPE,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Anchor => "anchor",
            Self::Peer => "peer",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryAdvertisement {
    pub role: DiscoveryRole,
    pub service_type: String,
    pub instance_name: String,
    pub host_name: String,
    pub port: u16,
    pub txt: BTreeMap<String, String>,
}

impl DiscoveryAdvertisement {
    pub fn to_service_info(&self) -> Result<ServiceInfo> {
        let properties: Vec<(&str, &str)> = self
            .txt
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect();
        ServiceInfo::new(
            self.service_type.as_str(),
            self.instance_name.as_str(),
            self.host_name.as_str(),
            "",
            self.port,
            &properties[..],
        )
        .map(ServiceInfo::enable_addr_auto)
        .map_err(|err| {
            DevRelayError::Config(format!("invalid mDNS discovery advertisement: {err}"))
        })
    }
}

pub struct DiscoveryService {
    daemon: ServiceDaemon,
}

impl DiscoveryService {
    pub fn new() -> Result<Self> {
        let daemon = ServiceDaemon::new()
            .map_err(|err| DevRelayError::Ipc(format!("failed to start mDNS daemon: {err}")))?;
        Ok(Self { daemon })
    }

    pub fn advertise(&self, advertisement: &DiscoveryAdvertisement) -> Result<()> {
        self.daemon
            .register(advertisement.to_service_info()?)
            .map_err(|err| DevRelayError::Ipc(format!("failed to advertise mDNS service: {err}")))
    }

    pub fn browse(&self, role: DiscoveryRole) -> Result<Receiver<ServiceEvent>> {
        self.daemon
            .browse(role.service_type())
            .map_err(|err| DevRelayError::Ipc(format!("failed to browse mDNS services: {err}")))
    }
}

pub fn build_discovery_advertisement(
    role: DiscoveryRole,
    fabric_id: &str,
    device_id: &str,
    port: u16,
) -> Result<DiscoveryAdvertisement> {
    validate_non_empty("fabric_id", fabric_id)?;
    validate_non_empty("device_id", device_id)?;
    if port == 0 {
        return Err(DevRelayError::Config(
            "discovery port must not be 0".to_string(),
        ));
    }

    let device_hint = public_identity_hint(device_id);
    let mut txt = BTreeMap::new();
    txt.insert(
        DISCOVERY_TXT_PROTOCOL.to_string(),
        DEVRELAY_DISCOVERY_PROTOCOL.to_string(),
    );
    txt.insert(
        DISCOVERY_TXT_FABRIC_HINT.to_string(),
        truncated_fabric_hint(fabric_id),
    );
    txt.insert(DISCOVERY_TXT_DEVICE_ID.to_string(), device_id.to_string());
    txt.insert(DISCOVERY_TXT_PORT.to_string(), port.to_string());

    Ok(DiscoveryAdvertisement {
        role,
        service_type: role.service_type().to_string(),
        instance_name: format!("devrelay-{}-{device_hint}", role.label()),
        host_name: format!("devrelay-{device_hint}.local."),
        port,
        txt,
    })
}

pub fn truncated_fabric_hint(fabric_id: &str) -> String {
    fabric_id
        .strip_prefix("f_")
        .unwrap_or(fabric_id)
        .chars()
        .take(12)
        .collect()
}

fn public_identity_hint(identity: &str) -> String {
    let hint: String = identity
        .strip_prefix("d_")
        .unwrap_or(identity)
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(12)
        .collect();
    if hint.is_empty() {
        let digest = blake3::hash(identity.as_bytes());
        digest.to_hex()[..12].to_string()
    } else {
        hint.to_ascii_lowercase()
    }
}

fn validate_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(DevRelayError::Config(format!("{field} must not be empty")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_anchor_and_peer_service_types() {
        let anchor = build_discovery_advertisement(
            DiscoveryRole::Anchor,
            "f_1234567890abcdef",
            "d_alpha",
            7001,
        )
        .unwrap();
        let peer = build_discovery_advertisement(
            DiscoveryRole::Peer,
            "f_1234567890abcdef",
            "d_alpha",
            7002,
        )
        .unwrap();

        assert_eq!(anchor.service_type, DEVRELAY_ANCHOR_SERVICE_TYPE);
        assert_eq!(peer.service_type, DEVRELAY_PEER_SERVICE_TYPE);
        assert_eq!(anchor.port, 7001);
        assert_eq!(peer.port, 7002);
    }

    #[test]
    fn txt_records_include_protocol_fabric_device_and_port() {
        let advertisement = build_discovery_advertisement(
            DiscoveryRole::Anchor,
            "f_1234567890abcdef",
            "d_public",
            7717,
        )
        .unwrap();

        assert_eq!(
            advertisement.txt[DISCOVERY_TXT_PROTOCOL],
            DEVRELAY_DISCOVERY_PROTOCOL
        );
        assert_eq!(advertisement.txt[DISCOVERY_TXT_FABRIC_HINT], "1234567890ab");
        assert_eq!(advertisement.txt[DISCOVERY_TXT_DEVICE_ID], "d_public");
        assert_eq!(advertisement.txt[DISCOVERY_TXT_PORT], "7717");
    }

    #[test]
    fn service_info_accepts_discovery_descriptor() {
        let advertisement = build_discovery_advertisement(
            DiscoveryRole::Peer,
            "f_abcdef1234567890",
            "d_peer",
            8123,
        )
        .unwrap();

        let service = advertisement.to_service_info().unwrap();

        assert_eq!(service.get_type(), DEVRELAY_PEER_SERVICE_TYPE);
        assert_eq!(service.get_port(), 8123);
        assert_eq!(service.get_property_val_str("protocol"), Some("1"));
        assert_eq!(service.get_property_val_str("device_id"), Some("d_peer"));
    }

    #[test]
    fn rejects_zero_port() {
        let err = build_discovery_advertisement(DiscoveryRole::Anchor, "f_abc", "d_device", 0)
            .unwrap_err();

        assert!(err.to_string().contains("port"));
    }

    #[test]
    fn txt_records_do_not_include_project_paths_or_usernames() {
        let advertisement = build_discovery_advertisement(
            DiscoveryRole::Anchor,
            "f_fabricdoesnotidentifyalice",
            "d_publicdevice",
            7000,
        )
        .unwrap();
        let keys: Vec<&str> = advertisement.txt.keys().map(String::as_str).collect();
        let encoded = serde_json::to_string(&advertisement).unwrap();

        assert_eq!(keys, ["device_id", "fabric", "port", "protocol"]);
        for forbidden in [
            "SecretProject",
            "/Users/alice/private/repo",
            "alice",
            "repository",
            "local_path",
        ] {
            assert!(
                !encoded.contains(forbidden),
                "discovery advertisement leaked {forbidden}: {encoded}"
            );
        }
    }
}
