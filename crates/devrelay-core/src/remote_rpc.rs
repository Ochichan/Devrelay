//! Remote Control RPC pre-dispatch policy.
//!
//! This module is transport-adjacent glue for the future M4.5 server. It does
//! not run a socket server; it centralizes the checks a remote JSON-RPC request
//! must pass before any method dispatch can happen.

use crate::{
    AuthenticatedControlPlanePeer, ControlPlaneReplayCache, ControlPlaneRequestEnvelope,
    ControlPlaneTransportPolicy, DevRelayError, DevicesListResult, MetadataDb,
    ProjectRegistryIndex, Result, RpcError, RpcRequest, RpcResponse, RpcVersionNegotiationParams,
    RpcVersionNegotiationResult, ValidatedDeviceCertificate, WorkspaceState,
    require_authenticated_control_channel, validate_control_request_envelope,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::rpc::{
    METHOD_DEVICES_LIST, METHOD_HANDOFF_ABORT, METHOD_HANDOFF_BEGIN, METHOD_HANDOFF_COMMIT,
    METHOD_HANDOFF_RECOVER, METHOD_HANDOFF_SOURCE_READY, METHOD_HANDOFF_TARGET_VERIFY,
    METHOD_HANDOFFS_LIST, METHOD_PROJECTS_LIST, METHOD_RPC_NEGOTIATE, RPC_INVALID_REQUEST,
    RPC_PROTOCOL_VERSION,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteProjectSummary {
    pub project_id: String,
    pub display_name: String,
    pub workspace_count: usize,
    pub remote_url_fingerprint: Option<String>,
    pub root_commit_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteProjectsListResult {
    pub projects: Vec<RemoteProjectSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWorkspacesListParams {
    pub project: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWorkspaceSummary {
    pub workspace_id: String,
    pub project_id: String,
    pub device_id: String,
    pub platform_profile: String,
    pub state: WorkspaceState,
    pub last_seen_head: Option<String>,
    pub last_checkpoint_id: Option<String>,
    pub local_path_redacted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteWorkspacesListResult {
    pub workspaces: Vec<RemoteWorkspaceSummary>,
}

pub fn is_remote_rpc_method_allowed(method: &str) -> bool {
    REMOTE_RPC_METHODS.contains(&method)
}

pub fn remote_rpc_negotiate(
    params: RpcVersionNegotiationParams,
) -> std::result::Result<RpcVersionNegotiationResult, RpcError> {
    if params.client_protocol_version != RPC_PROTOCOL_VERSION {
        return Err(RpcError::version_mismatch(params.client_protocol_version));
    }
    Ok(RpcVersionNegotiationResult {
        protocol_version: RPC_PROTOCOL_VERSION,
        server_name: "devrelay-remote".to_string(),
        methods: REMOTE_RPC_METHODS
            .iter()
            .map(|method| (*method).to_string())
            .collect(),
    })
}

pub fn remote_devices_list(metadata: &MetadataDb) -> Result<DevicesListResult> {
    Ok(DevicesListResult {
        devices: metadata.list_devices()?,
    })
}

pub fn remote_projects_list(registry: &ProjectRegistryIndex) -> RemoteProjectsListResult {
    RemoteProjectsListResult {
        projects: registry
            .projects
            .values()
            .map(|project| RemoteProjectSummary {
                project_id: project.project_id.clone(),
                display_name: project.display_name.clone(),
                workspace_count: project.workspaces.len(),
                remote_url_fingerprint: project.remote_url_fingerprint.clone(),
                root_commit_fingerprint: project.root_commit_fingerprint.clone(),
            })
            .collect(),
    }
}

pub fn remote_workspaces_list(
    registry: &ProjectRegistryIndex,
    params: RemoteWorkspacesListParams,
) -> Result<RemoteWorkspacesListResult> {
    let project = registry
        .projects
        .get(&params.project)
        .ok_or_else(|| DevRelayError::Config(format!("unknown project {}", params.project)))?;
    Ok(RemoteWorkspacesListResult {
        workspaces: project
            .workspaces
            .values()
            .map(|workspace| RemoteWorkspaceSummary {
                workspace_id: workspace.workspace_id.clone(),
                project_id: workspace.project_id.clone(),
                device_id: workspace.device_id.clone(),
                platform_profile: workspace.platform_profile.clone(),
                state: workspace.state,
                last_seen_head: workspace.last_seen_head.clone(),
                last_checkpoint_id: workspace.last_checkpoint_id.clone(),
                local_path_redacted: true,
            })
            .collect(),
    })
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
    use crate::{
        CONTROL_PROTOCOL_VERSION, ControlPlaneTransportSecurity, DeviceIdentity,
        ProjectRegistryEntry, WorkspaceRegistryEntry,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

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

    #[test]
    fn remote_rpc_negotiate_returns_remote_method_catalog() {
        let result = remote_rpc_negotiate(RpcVersionNegotiationParams {
            client_protocol_version: RPC_PROTOCOL_VERSION,
        })
        .unwrap();

        assert_eq!(result.server_name, "devrelay-remote");
        assert_eq!(result.protocol_version, RPC_PROTOCOL_VERSION);
        assert_eq!(
            result.methods,
            REMOTE_RPC_METHODS
                .iter()
                .map(|method| (*method).to_string())
                .collect::<Vec<_>>()
        );

        let mismatch = remote_rpc_negotiate(RpcVersionNegotiationParams {
            client_protocol_version: RPC_PROTOCOL_VERSION + 1,
        })
        .unwrap_err();
        assert_eq!(mismatch.code, crate::rpc::RPC_VERSION_MISMATCH);
    }

    #[test]
    fn remote_devices_list_reads_metadata_devices() {
        let temp = tempfile::tempdir().unwrap();
        let db = MetadataDb::open(temp.path().join("metadata.db")).unwrap();
        db.upsert_device_identity(&DeviceIdentity {
            device_id: "device-a".to_string(),
            display_name: "MacBook".to_string(),
            platform_key: "darwin-arm64".to_string(),
            architecture: "arm64".to_string(),
            capabilities_json: "{}".to_string(),
            paired_at_unix_seconds: Some(1_700_000_000),
            last_seen_unix_seconds: 1_700_000_100,
        })
        .unwrap();

        let result = remote_devices_list(&db).unwrap();

        assert_eq!(result.devices.len(), 1);
        assert_eq!(result.devices[0].device_id, "device-a");
    }

    #[test]
    fn remote_projects_list_redacts_local_paths() {
        let registry = registry();

        let result = remote_projects_list(&registry);
        let encoded = serde_json::to_value(&result).unwrap();

        assert_eq!(result.projects[0].project_id, "project-a");
        assert_eq!(result.projects[0].workspace_count, 1);
        assert_eq!(
            result.projects[0].remote_url_fingerprint.as_deref(),
            Some("remote-fp")
        );
        assert!(encoded["projects"][0].get("local_path").is_none());
        assert!(
            !serde_json::to_string(&encoded)
                .unwrap()
                .contains("/private/repo")
        );
    }

    #[test]
    fn remote_workspaces_list_redacts_local_paths() {
        let registry = registry();

        let result = remote_workspaces_list(
            &registry,
            RemoteWorkspacesListParams {
                project: "project-a".to_string(),
            },
        )
        .unwrap();
        let encoded = serde_json::to_value(&result).unwrap();

        assert_eq!(result.workspaces.len(), 1);
        assert_eq!(result.workspaces[0].workspace_id, "w_project_device_path");
        assert!(result.workspaces[0].local_path_redacted);
        assert!(encoded["workspaces"][0].get("local_path").is_none());
        assert!(
            !serde_json::to_string(&encoded)
                .unwrap()
                .contains("/private/repo")
        );
    }

    #[test]
    fn remote_workspaces_list_rejects_unknown_project() {
        let err = remote_workspaces_list(
            &registry(),
            RemoteWorkspacesListParams {
                project: "missing".to_string(),
            },
        )
        .unwrap_err();

        assert!(err.to_string().contains("unknown project missing"));
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

    fn registry() -> ProjectRegistryIndex {
        let workspace = WorkspaceRegistryEntry {
            workspace_id: "w_project_device_path".to_string(),
            project_id: "project-a".to_string(),
            device_id: "device-a".to_string(),
            local_path: PathBuf::from("/private/repo"),
            platform_profile: "darwin-arm64".to_string(),
            state: WorkspaceState::Active,
            last_seen_head: Some("abc123".to_string()),
            last_checkpoint_id: Some("s1_000000000000000000000001".to_string()),
        };
        ProjectRegistryIndex {
            projects: BTreeMap::from([(
                "project-a".to_string(),
                ProjectRegistryEntry {
                    project_id: "project-a".to_string(),
                    display_name: "Demo".to_string(),
                    local_path: PathBuf::from("/private/repo"),
                    workspaces: BTreeMap::from([(workspace.workspace_id.clone(), workspace)]),
                    manifest_path: Some(PathBuf::from("/private/repo/devrelay.toml")),
                    remote_url_fingerprint: Some("remote-fp".to_string()),
                    root_commit_fingerprint: Some("root-fp".to_string()),
                },
            )]),
        }
    }
}
