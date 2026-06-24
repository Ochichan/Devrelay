//! Remote Control RPC pre-dispatch policy.
//!
//! This module is transport-adjacent glue for the future M4.5 server. It does
//! not run a socket server; it centralizes the checks a remote JSON-RPC request
//! must pass before any method dispatch can happen.

use crate::{
    AuthenticatedControlPlanePeer, ControlPlaneReplayCache, ControlPlaneRequestEnvelope,
    ControlPlaneTransportPolicy, DevRelayError, RpcError, RpcRequest, RpcResponse,
    ValidatedDeviceCertificate, require_authenticated_control_channel,
    validate_control_request_envelope,
};
use serde_json::json;

use crate::rpc::{
    METHOD_DEVICES_LIST, METHOD_HANDOFF_ABORT, METHOD_HANDOFF_BEGIN, METHOD_HANDOFF_COMMIT,
    METHOD_HANDOFF_RECOVER, METHOD_HANDOFF_SOURCE_READY, METHOD_HANDOFF_TARGET_VERIFY,
    METHOD_HANDOFFS_LIST, METHOD_PROJECTS_LIST, METHOD_RPC_NEGOTIATE, RPC_INVALID_REQUEST,
};

pub const METHOD_REMOTE_WORKSPACES_LIST: &str = "workspaces.list";
pub const METHOD_REMOTE_SESSIONS_SNAPSHOTS_LIST: &str = "sessions.snapshots.list";
pub const METHOD_REMOTE_RECOVERY_LIST: &str = "recovery.list";
pub const METHOD_REMOTE_RECOVERY_OPEN: &str = "recovery.open";

pub const REMOTE_RPC_METHODS: &[&str] = &[
    METHOD_RPC_NEGOTIATE,
    METHOD_DEVICES_LIST,
    METHOD_PROJECTS_LIST,
    METHOD_REMOTE_WORKSPACES_LIST,
    METHOD_REMOTE_SESSIONS_SNAPSHOTS_LIST,
    METHOD_HANDOFFS_LIST,
    METHOD_HANDOFF_BEGIN,
    METHOD_HANDOFF_ABORT,
    METHOD_HANDOFF_TARGET_VERIFY,
    METHOD_HANDOFF_SOURCE_READY,
    METHOD_HANDOFF_COMMIT,
    METHOD_HANDOFF_RECOVER,
    METHOD_REMOTE_RECOVERY_LIST,
    METHOD_REMOTE_RECOVERY_OPEN,
];

#[derive(Debug, Clone, PartialEq)]
pub struct RemoteRpcRequestContext {
    pub peer: AuthenticatedControlPlanePeer,
    pub request: RpcRequest,
}

pub fn is_remote_rpc_method_allowed(method: &str) -> bool {
    REMOTE_RPC_METHODS.contains(&method)
}

pub fn preflight_remote_rpc_request(
    peer: Option<ValidatedDeviceCertificate>,
    control_envelope: &ControlPlaneRequestEnvelope,
    rpc_bytes: &[u8],
    policy: &ControlPlaneTransportPolicy,
    now_unix_seconds: u64,
    replay_cache: &mut ControlPlaneReplayCache,
) -> std::result::Result<RemoteRpcRequestContext, RpcResponse> {
    let peer = require_authenticated_control_channel(peer)
        .map_err(|err| RpcResponse::error(None, remote_rpc_error_from_devrelay(err)))?;
    validate_control_request_envelope(policy, control_envelope, now_unix_seconds, replay_cache)
        .map_err(|err| RpcResponse::error(None, remote_rpc_error_from_devrelay(err)))?;
    let request = RpcRequest::parse(rpc_bytes).map_err(|err| RpcResponse::error(None, err))?;
    let request_id = request
        .required_id()
        .map_err(|err| RpcResponse::error(request.id.clone(), err))?;
    if !is_remote_rpc_method_allowed(&request.method) {
        return Err(RpcResponse::error(
            Some(request_id),
            RpcError::method_not_found(&request.method),
        ));
    }
    Ok(RemoteRpcRequestContext { peer, request })
}

pub fn remote_rpc_error_from_devrelay(error: DevRelayError) -> RpcError {
    let info = error.info();
    RpcError {
        code: RPC_INVALID_REQUEST,
        message: info.title.to_string(),
        data: Some(json!({
            "code": info.code,
            "detail": info.detail,
            "safe_actions": info.safe_actions,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rpc::{RPC_INVALID_REQUEST, RPC_METHOD_NOT_FOUND};
    use crate::{CONTROL_PROTOCOL_VERSION, ControlPlaneTransportSecurity};

    #[test]
    fn remote_rpc_preflight_requires_authenticated_peer_before_request_parse() {
        let mut cache = ControlPlaneReplayCache::new();
        let response = preflight_remote_rpc_request(
            None,
            &envelope("nonce_auth_required"),
            b"not json",
            &ControlPlaneTransportPolicy::default(),
            1_000,
            &mut cache,
        )
        .unwrap_err();

        assert!(response.id.is_none());
        let error = response.error.unwrap();
        assert_eq!(error.code, RPC_INVALID_REQUEST);
        assert_eq!(error.data.unwrap()["code"], "DR-CONFIG");
    }

    #[test]
    fn remote_rpc_preflight_requires_request_id_and_allowlisted_method() {
        let mut cache = ControlPlaneReplayCache::new();
        let missing_id = preflight_remote_rpc_request(
            Some(peer()),
            &envelope("nonce_missing_id"),
            br#"{"jsonrpc":"2.0","method":"devices.list","params":{}}"#,
            &ControlPlaneTransportPolicy::default(),
            1_000,
            &mut cache,
        )
        .unwrap_err();
        assert!(missing_id.id.is_none());
        assert_eq!(missing_id.error.unwrap().code, RPC_INVALID_REQUEST);

        let forbidden = preflight_remote_rpc_request(
            Some(peer()),
            &envelope("nonce_forbidden_method"),
            br#"{"jsonrpc":"2.0","id":"abc","method":"settings.get","params":{}}"#,
            &ControlPlaneTransportPolicy::default(),
            1_000,
            &mut cache,
        )
        .unwrap_err();
        assert_eq!(forbidden.id, Some(crate::RpcId::String("abc".to_string())));
        assert_eq!(forbidden.error.unwrap().code, RPC_METHOD_NOT_FOUND);
    }

    #[test]
    fn remote_rpc_preflight_accepts_allowlisted_method() {
        let mut cache = ControlPlaneReplayCache::new();
        let context = preflight_remote_rpc_request(
            Some(peer()),
            &envelope("nonce_allowed_method"),
            br#"{"jsonrpc":"2.0","id":"abc","method":"handoffs.list","params":{"project":"12345678"}}"#,
            &ControlPlaneTransportPolicy::default(),
            1_000,
            &mut cache,
        )
        .unwrap();

        assert_eq!(context.peer.transport, ControlPlaneTransportSecurity::Mtls);
        assert_eq!(context.peer.device.device_id, "device-a");
        assert_eq!(context.request.method, METHOD_HANDOFFS_LIST);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn remote_rpc_allowlist_matches_first_schema_methods() {
        for method in [
            METHOD_RPC_NEGOTIATE,
            METHOD_DEVICES_LIST,
            METHOD_PROJECTS_LIST,
            METHOD_REMOTE_WORKSPACES_LIST,
            METHOD_REMOTE_SESSIONS_SNAPSHOTS_LIST,
            METHOD_HANDOFFS_LIST,
            METHOD_HANDOFF_BEGIN,
            METHOD_HANDOFF_ABORT,
            METHOD_HANDOFF_TARGET_VERIFY,
            METHOD_HANDOFF_SOURCE_READY,
            METHOD_HANDOFF_COMMIT,
            METHOD_HANDOFF_RECOVER,
            METHOD_REMOTE_RECOVERY_LIST,
            METHOD_REMOTE_RECOVERY_OPEN,
        ] {
            assert!(is_remote_rpc_method_allowed(method), "{method}");
        }
    }

    fn envelope(nonce: &str) -> ControlPlaneRequestEnvelope {
        ControlPlaneRequestEnvelope {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            sent_at_unix_seconds: 1_000,
            replay_nonce: nonce.to_string(),
        }
    }

    fn peer() -> ValidatedDeviceCertificate {
        ValidatedDeviceCertificate {
            certificate_id: "cert_test".to_string(),
            fabric_id: "fabric_test".to_string(),
            device_id: "device-a".to_string(),
            signing_public_key_hex: "a".repeat(64),
            network_public_key_hex: "b".repeat(64),
            expires_at_unix_seconds: 2_000,
        }
    }
}
