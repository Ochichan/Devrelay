//! Remote Control RPC transport: framed control messages over mTLS.
//!
//! One request is one length-prefixed JSON frame holding the control envelope
//! (protocol version, timestamp, replay nonce), the caller's fabric-issued
//! device certificate, and the JSON-RPC request itself. Responses are plain
//! length-prefixed JSON-RPC responses. A connection may carry many requests.
//!
//! The server side authenticates every frame: the TLS layer already verified
//! that the peer certificate chains to the fabric X.509 CA, and
//! [`authenticate_remote_control_frame`] then validates the application-level
//! device certificate (pinned fabric root, expiry, revocation, signature),
//! binds the TLS peer key to that certificate, and runs the shared remote RPC
//! preflight (allowlist, envelope, replay, request id).

use crate::{
    ControlPlaneReplayCache, ControlPlaneRequestEnvelope, ControlPlaneTransportPolicy,
    DEVRELAY_CONTROL_SERVER_NAME, DevRelayError, DeviceCertificate, DeviceRevocationRecord,
    FabricRootIdentity, IpcLimits, RemoteRpcRequestContext, Result, RpcError, RpcId, RpcRequest,
    RpcResponse, extract_single_ed25519_spki, preflight_remote_rpc_request, read_framed_message,
    remote_rpc_error_from_devrelay, unix_now_seconds, validate_device_certificate,
    write_framed_message,
};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;

/// One remote control request as sent on the wire.
#[derive(Debug, Serialize, Deserialize)]
pub struct RemoteControlFrame {
    pub control: ControlPlaneRequestEnvelope,
    pub device_certificate: DeviceCertificate,
    pub rpc: Box<RawValue>,
}

/// Authenticates one framed remote control request before dispatch.
///
/// `tls_peer_leaf_der` must be the leaf certificate the TLS layer verified
/// against the fabric X.509 CA. Errors come back as ready-to-send JSON-RPC
/// responses so transport code never invents its own error mapping.
pub fn authenticate_remote_control_frame(
    frame_bytes: &[u8],
    tls_peer_leaf_der: &[u8],
    pinned_root: &FabricRootIdentity,
    revocations: &[DeviceRevocationRecord],
    policy: &ControlPlaneTransportPolicy,
    now_unix_seconds: u64,
    replay_cache: &mut ControlPlaneReplayCache,
) -> std::result::Result<RemoteRpcRequestContext, Box<RpcResponse>> {
    let frame: RemoteControlFrame = serde_json::from_slice(frame_bytes).map_err(|err| {
        RpcResponse::error(
            None,
            RpcError::invalid_request(format!("malformed remote control frame: {err}")),
        )
    })?;
    let validated = validate_device_certificate(
        &frame.device_certificate,
        pinned_root,
        revocations,
        now_unix_seconds,
    )
    .map_err(|err| RpcResponse::error(None, remote_rpc_error_from_devrelay(err)))?;
    let tls_peer_key = extract_single_ed25519_spki(tls_peer_leaf_der)
        .map_err(|err| RpcResponse::error(None, remote_rpc_error_from_devrelay(err)))?;
    if hex_encode(&tls_peer_key) != validated.signing_public_key_hex {
        return Err(Box::new(RpcResponse::error(
            None,
            RpcError::invalid_request(
                "device certificate does not match the authenticated TLS channel key",
            ),
        )));
    }
    preflight_remote_rpc_request(
        Some(validated),
        &frame.control,
        frame.rpc.get().as_bytes(),
        policy,
        now_unix_seconds,
        replay_cache,
    )
}

/// Minimal synchronous remote control client over mTLS.
///
/// Used by integration tests and by devices driving a paired agent. The
/// connection binds to a specific server key when one is expected, so a
/// different fabric device cannot silently answer for the target.
pub struct RemoteControlClient {
    stream: StreamOwned<ClientConnection, TcpStream>,
    limits: IpcLimits,
    device_certificate: DeviceCertificate,
    protocol_version: u32,
    next_request_id: u64,
}

impl RemoteControlClient {
    pub fn connect(
        address: SocketAddr,
        tls_config: Arc<ClientConfig>,
        device_certificate: DeviceCertificate,
        expected_server_signing_public_key_hex: Option<&str>,
        policy: &ControlPlaneTransportPolicy,
        limits: IpcLimits,
    ) -> Result<Self> {
        let tcp = TcpStream::connect_timeout(
            &address,
            std::time::Duration::from_millis(policy.connection_timeout_millis),
        )?;
        tcp.set_read_timeout(Some(std::time::Duration::from_millis(
            policy.request_timeout_millis,
        )))?;
        tcp.set_write_timeout(Some(std::time::Duration::from_millis(
            policy.request_timeout_millis,
        )))?;
        tcp.set_nodelay(true)?;
        let server_name = ServerName::try_from(DEVRELAY_CONTROL_SERVER_NAME)
            .map_err(|err| DevRelayError::Config(format!("invalid control server name: {err}")))?;
        let connection = ClientConnection::new(tls_config, server_name)
            .map_err(|err| DevRelayError::Config(format!("failed to start TLS client: {err}")))?;
        let mut stream = StreamOwned::new(connection, tcp);
        while stream.conn.is_handshaking() {
            stream.conn.complete_io(&mut stream.sock).map_err(|err| {
                DevRelayError::Config(format!("control TLS handshake failed: {err}"))
            })?;
        }
        if let Some(expected) = expected_server_signing_public_key_hex {
            let leaf = stream
                .conn
                .peer_certificates()
                .and_then(|certificates| certificates.first())
                .ok_or_else(|| {
                    DevRelayError::Config("control server presented no TLS certificate".to_string())
                })?;
            let server_key = extract_single_ed25519_spki(leaf.as_ref())?;
            if hex_encode(&server_key) != expected {
                return Err(DevRelayError::Config(
                    "control server TLS key does not match the expected device".to_string(),
                ));
            }
        }
        Ok(Self {
            stream,
            limits,
            device_certificate,
            protocol_version: policy.protocol_version,
            next_request_id: 1,
        })
    }

    /// Sends one remote JSON-RPC request and reads its response.
    pub fn call(&mut self, method: &str, params: serde_json::Value) -> Result<RpcResponse> {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        let request = RpcRequest {
            jsonrpc: crate::RPC_JSONRPC_VERSION.to_string(),
            id: Some(RpcId::String(format!("remote-{request_id}"))),
            method: method.to_string(),
            params,
        };
        let frame = RemoteControlFrame {
            control: ControlPlaneRequestEnvelope {
                protocol_version: self.protocol_version,
                sent_at_unix_seconds: unix_now_seconds(),
                replay_nonce: generate_replay_nonce()?,
            },
            device_certificate: self.device_certificate.clone(),
            rpc: serde_json::value::to_raw_value(&request)?,
        };
        let frame_bytes = serde_json::to_vec(&frame)?;
        write_framed_message(&mut self.stream, &frame_bytes, self.limits)?;
        let response_bytes = read_framed_message(&mut self.stream, self.limits)?;
        Ok(serde_json::from_slice(&response_bytes)?)
    }
}

fn generate_replay_nonce() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|err| DevRelayError::Config(format!("failed to read OS entropy: {err}")))?;
    Ok(hex_encode(&bytes))
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
    use crate::{
        CONTROL_PROTOCOL_VERSION, DevRelayHome, DevicePublicIdentity, FabricIdentityStore,
        LocalConfig, METHOD_DEVICES_LIST,
    };
    use serde_json::json;

    struct Fixture {
        _temp: tempfile::TempDir,
        store: FabricIdentityStore,
        root: FabricRootIdentity,
        device_certificate: DeviceCertificate,
        leaf_der: Vec<u8>,
    }

    fn fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path());
        let config = LocalConfig::new_for_local_device();
        let store = FabricIdentityStore::new(home);
        let bundle = store.open_or_create(&config).unwrap();
        let device_certificate = store
            .issue_device_certificate(&bundle.device, 1_000, 3_600)
            .unwrap();
        let leaf_der = store
            .device_tls_identity(&config.device_id)
            .unwrap()
            .cert_chain_der
            .remove(0);
        Fixture {
            _temp: temp,
            store,
            root: bundle.root,
            device_certificate,
            leaf_der,
        }
    }

    fn frame_bytes(fixture: &Fixture, nonce: &str, rpc: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&RemoteControlFrame {
            control: ControlPlaneRequestEnvelope {
                protocol_version: CONTROL_PROTOCOL_VERSION,
                sent_at_unix_seconds: 1_000,
                replay_nonce: nonce.to_string(),
            },
            device_certificate: fixture.device_certificate.clone(),
            rpc: serde_json::value::to_raw_value(&rpc).unwrap(),
        })
        .unwrap()
    }

    fn request_json(method: &str) -> serde_json::Value {
        json!({"jsonrpc": "2.0", "id": "t-1", "method": method, "params": {}})
    }

    #[test]
    fn authenticate_accepts_bound_frame_and_returns_context() {
        let fixture = fixture();
        let mut cache = ControlPlaneReplayCache::new();

        let context = authenticate_remote_control_frame(
            &frame_bytes(
                &fixture,
                "nonce_accept_00001",
                request_json(METHOD_DEVICES_LIST),
            ),
            &fixture.leaf_der,
            &fixture.root,
            &[],
            &ControlPlaneTransportPolicy::default(),
            1_000,
            &mut cache,
        )
        .unwrap();

        assert_eq!(
            context.peer.device.device_id,
            fixture.device_certificate.device_id
        );
        assert_eq!(context.request.method, METHOD_DEVICES_LIST);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn authenticate_rejects_certificate_not_matching_tls_channel_key() {
        let fixture = fixture();
        let other_secret = [9u8; 32];
        let other_public = ed25519_dalek::SigningKey::from_bytes(&other_secret)
            .verifying_key()
            .to_bytes();
        let other_leaf = fixture
            .store
            .issue_peer_tls_certificate_der("other-device", &hex_encode(&other_public))
            .unwrap();

        let response = authenticate_remote_control_frame(
            &frame_bytes(
                &fixture,
                "nonce_mismatch_001",
                request_json(METHOD_DEVICES_LIST),
            ),
            &other_leaf,
            &fixture.root,
            &[],
            &ControlPlaneTransportPolicy::default(),
            1_000,
            &mut ControlPlaneReplayCache::new(),
        )
        .unwrap_err();

        let error = response.error.unwrap();
        assert!(
            error.data.as_ref().unwrap()["detail"]
                .as_str()
                .unwrap()
                .contains("TLS channel key")
        );
    }

    #[test]
    fn authenticate_rejects_revoked_and_expired_certificates() {
        let fixture = fixture();
        let revocation = DeviceRevocationRecord {
            device_id: fixture.device_certificate.device_id.clone(),
            revoked_by_device_id: "security".to_string(),
            reason: "lost".to_string(),
            key_rotation_required: false,
            revoked_at_unix_seconds: 900,
        };

        let revoked = authenticate_remote_control_frame(
            &frame_bytes(
                &fixture,
                "nonce_revoked_0001",
                request_json(METHOD_DEVICES_LIST),
            ),
            &fixture.leaf_der,
            &fixture.root,
            std::slice::from_ref(&revocation),
            &ControlPlaneTransportPolicy::default(),
            1_000,
            &mut ControlPlaneReplayCache::new(),
        )
        .unwrap_err();
        assert!(
            revoked.error.as_ref().unwrap().data.as_ref().unwrap()["detail"]
                .as_str()
                .unwrap()
                .contains("revoked")
        );

        let expired = authenticate_remote_control_frame(
            &frame_bytes(
                &fixture,
                "nonce_expired_0001",
                request_json(METHOD_DEVICES_LIST),
            ),
            &fixture.leaf_der,
            &fixture.root,
            &[],
            &ControlPlaneTransportPolicy::default(),
            1_000_000,
            &mut ControlPlaneReplayCache::new(),
        )
        .unwrap_err();
        assert!(
            expired.error.as_ref().unwrap().data.as_ref().unwrap()["detail"]
                .as_str()
                .unwrap()
                .contains("expired")
        );
    }

    #[test]
    fn authenticate_rejects_replayed_nonce_and_disallowed_method() {
        let fixture = fixture();
        let mut cache = ControlPlaneReplayCache::new();
        let bytes = frame_bytes(
            &fixture,
            "nonce_replay_00001",
            request_json(METHOD_DEVICES_LIST),
        );

        authenticate_remote_control_frame(
            &bytes,
            &fixture.leaf_der,
            &fixture.root,
            &[],
            &ControlPlaneTransportPolicy::default(),
            1_000,
            &mut cache,
        )
        .unwrap();
        let replayed = authenticate_remote_control_frame(
            &bytes,
            &fixture.leaf_der,
            &fixture.root,
            &[],
            &ControlPlaneTransportPolicy::default(),
            1_001,
            &mut cache,
        )
        .unwrap_err();
        assert!(
            replayed.error.as_ref().unwrap().data.as_ref().unwrap()["detail"]
                .as_str()
                .unwrap()
                .contains("already used")
        );

        let forbidden = authenticate_remote_control_frame(
            &frame_bytes(
                &fixture,
                "nonce_forbidden_01",
                request_json("settings.update"),
            ),
            &fixture.leaf_der,
            &fixture.root,
            &[],
            &ControlPlaneTransportPolicy::default(),
            1_000,
            &mut cache,
        )
        .unwrap_err();
        assert_eq!(
            forbidden.error.as_ref().unwrap().code,
            crate::rpc::RPC_METHOD_NOT_FOUND
        );
    }

    #[test]
    fn authenticate_rejects_frame_from_wrong_fabric() {
        let fixture = fixture();
        let other_temp = tempfile::tempdir().unwrap();
        let other_home = DevRelayHome::new(other_temp.path());
        let other_config = LocalConfig::new_for_local_device();
        let other_store = FabricIdentityStore::new(other_home);
        let other_bundle = other_store.open_or_create(&other_config).unwrap();
        let mut wrong_device: DevicePublicIdentity = other_bundle.device.clone();
        wrong_device.fabric_id = other_bundle.root.fabric_id.clone();
        let wrong_certificate = other_store
            .issue_device_certificate(&wrong_device, 1_000, 3_600)
            .unwrap();

        let frame = serde_json::to_vec(&RemoteControlFrame {
            control: ControlPlaneRequestEnvelope {
                protocol_version: CONTROL_PROTOCOL_VERSION,
                sent_at_unix_seconds: 1_000,
                replay_nonce: "nonce_wrongfab_001".to_string(),
            },
            device_certificate: wrong_certificate,
            rpc: serde_json::value::to_raw_value(&request_json(METHOD_DEVICES_LIST)).unwrap(),
        })
        .unwrap();

        let response = authenticate_remote_control_frame(
            &frame,
            &fixture.leaf_der,
            &fixture.root,
            &[],
            &ControlPlaneTransportPolicy::default(),
            1_000,
            &mut ControlPlaneReplayCache::new(),
        )
        .unwrap_err();
        assert!(
            response.error.as_ref().unwrap().data.as_ref().unwrap()["detail"]
                .as_str()
                .unwrap()
                .contains("pinned fabric")
        );
    }
}
