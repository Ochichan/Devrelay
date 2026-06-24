//! Remote Control RPC pre-dispatch policy.
//!
//! This module is transport-adjacent glue for the future M4.5 server. It does
//! not run a socket server; it centralizes the checks a remote JSON-RPC request
//! must pass before any method dispatch can happen.

use crate::{
    AuthenticatedControlPlanePeer, ControlPlaneReplayCache, ControlPlaneRequestEnvelope,
    ControlPlaneTransportPolicy, DevRelayError, DevicesListResult, HandoffBeginParams,
    HandoffCommitParams, HandoffIdParams, HandoffMutationResult, HandoffRecord,
    HandoffRecoverParams, HandoffRecoverResult, HandoffStatus, HandoffsListParams,
    HandoffsListResult, MetadataDb, ProjectRegistryIndex, Result, RpcError, RpcRequest,
    RpcResponse, RpcVersionNegotiationParams, RpcVersionNegotiationResult, StoredSnapshot,
    ValidatedDeviceCertificate, WorkspaceState, require_authenticated_control_channel,
    validate_control_request_envelope,
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
pub const DEFAULT_REMOTE_SNAPSHOT_LIST_LIMIT: usize = 100;
pub const MAX_REMOTE_SNAPSHOT_LIST_LIMIT: usize = 500;
pub const DEFAULT_REMOTE_HANDOFF_TTL_SECONDS: u64 = 600;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSessionsSnapshotsListParams {
    pub project: String,
    pub session_id: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSnapshotSummary {
    pub snapshot_id: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
    pub sequence_number: i64,
    pub pinned: bool,
    pub label: Option<String>,
    pub created_at_unix_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteSessionsSnapshotsListResult {
    pub snapshots: Vec<RemoteSnapshotSummary>,
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

pub fn remote_sessions_snapshots_list(
    metadata: &MetadataDb,
    params: RemoteSessionsSnapshotsListParams,
) -> Result<RemoteSessionsSnapshotsListResult> {
    let mut snapshots = metadata.list_stored_snapshots(Some(&params.project))?;
    if let Some(session_id) = params.session_id.as_deref() {
        snapshots.retain(|snapshot| snapshot.session_id.as_deref() == Some(session_id));
    }
    snapshots.sort_by(|left, right| {
        right
            .sequence_number
            .cmp(&left.sequence_number)
            .then(
                right
                    .created_at_unix_seconds
                    .cmp(&left.created_at_unix_seconds),
            )
            .then(right.snapshot_id.cmp(&left.snapshot_id))
    });
    snapshots.truncate(
        params
            .limit
            .unwrap_or(DEFAULT_REMOTE_SNAPSHOT_LIST_LIMIT)
            .min(MAX_REMOTE_SNAPSHOT_LIST_LIMIT),
    );
    Ok(RemoteSessionsSnapshotsListResult {
        snapshots: snapshots
            .into_iter()
            .map(remote_snapshot_summary_from)
            .collect(),
    })
}

fn remote_snapshot_summary_from(snapshot: StoredSnapshot) -> RemoteSnapshotSummary {
    RemoteSnapshotSummary {
        snapshot_id: snapshot.snapshot_id,
        project_id: snapshot.project_id,
        session_id: snapshot.session_id,
        parent_snapshot_id: snapshot.parent_snapshot_id,
        sequence_number: snapshot.sequence_number,
        pinned: snapshot.pinned,
        label: snapshot.label,
        created_at_unix_seconds: snapshot.created_at_unix_seconds,
    }
}

pub fn remote_handoffs_list(
    metadata: &MetadataDb,
    params: HandoffsListParams,
) -> Result<HandoffsListResult> {
    let handoffs = metadata
        .list_handoffs(params.project.as_deref())?
        .into_iter()
        .map(|record| {
            let journal = if params.include_journal {
                metadata.list_handoff_journal(&record.handoff_id)?
            } else {
                Vec::new()
            };
            Ok(HandoffStatus { record, journal })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(HandoffsListResult { handoffs })
}

pub fn remote_handoff_begin(
    metadata: &mut MetadataDb,
    actor_device_id: &str,
    params: HandoffBeginParams,
) -> Result<HandoffMutationResult> {
    let lease = metadata
        .get_lease(&params.lease_id)?
        .ok_or_else(|| DevRelayError::Config(format!("unknown lease {}", params.lease_id)))?;
    require_project(&lease.project_id, &params.project, "handoff.begin")?;
    require_actor(
        lease.holder_device_id.as_deref(),
        actor_device_id,
        "handoff.begin source",
    )?;
    let handoff = metadata.begin_handoff(
        &params.lease_id,
        actor_device_id,
        &params.target_device_id,
        &params.source_generation,
        params
            .ttl_seconds
            .unwrap_or(DEFAULT_REMOTE_HANDOFF_TTL_SECONDS),
    )?;
    handoff_mutation_result(metadata, handoff)
}

pub fn remote_handoff_target_verify(
    metadata: &mut MetadataDb,
    actor_device_id: &str,
    params: HandoffIdParams,
) -> Result<HandoffMutationResult> {
    let handoff = require_handoff_project(metadata, &params.project, &params.handoff_id)?;
    require_actor(
        Some(&handoff.target_device_id),
        actor_device_id,
        "handoff.target.verify target",
    )?;
    let handoff = metadata.mark_handoff_target_verified(&params.handoff_id)?;
    handoff_mutation_result(metadata, handoff)
}

pub fn remote_handoff_source_ready(
    metadata: &mut MetadataDb,
    actor_device_id: &str,
    params: HandoffIdParams,
) -> Result<HandoffMutationResult> {
    let handoff = require_handoff_project(metadata, &params.project, &params.handoff_id)?;
    require_actor(
        Some(&handoff.source_device_id),
        actor_device_id,
        "handoff.source.ready source",
    )?;
    let handoff = metadata.mark_handoff_source_ready(&params.handoff_id)?;
    handoff_mutation_result(metadata, handoff)
}

pub fn remote_handoff_commit(
    metadata: &mut MetadataDb,
    actor_device_id: &str,
    params: HandoffCommitParams,
    now_unix_seconds: u64,
) -> Result<HandoffMutationResult> {
    let handoff = require_handoff_project(metadata, &params.project, &params.handoff_id)?;
    require_actor(
        Some(&handoff.source_device_id),
        actor_device_id,
        "handoff.commit source",
    )?;
    let handoff = metadata.commit_handoff(
        &params.handoff_id,
        &params.observed_source_generation,
        now_unix_seconds,
    )?;
    handoff_mutation_result(metadata, handoff)
}

pub fn remote_handoff_abort(
    metadata: &mut MetadataDb,
    actor_device_id: &str,
    params: HandoffIdParams,
) -> Result<HandoffMutationResult> {
    let handoff = require_handoff_project(metadata, &params.project, &params.handoff_id)?;
    require_source_or_target_actor(&handoff, actor_device_id, "handoff.abort")?;
    let handoff = metadata.abort_handoff(&params.handoff_id)?;
    handoff_mutation_result(metadata, handoff)
}

pub fn remote_handoff_recover(
    metadata: &mut MetadataDb,
    actor_device_id: &str,
    params: HandoffRecoverParams,
    now_unix_seconds: u64,
) -> Result<HandoffRecoverResult> {
    let handoff = require_handoff_project(metadata, &params.project, &params.handoff_id)?;
    require_source_or_target_actor(&handoff, actor_device_id, "handoff.recover")?;
    let outcome = metadata.recover_handoff(
        &params.handoff_id,
        &params.observed_source_generation,
        now_unix_seconds,
    )?;
    let handoff = require_handoff_project(metadata, &params.project, &params.handoff_id)?;
    let journal = metadata.list_handoff_journal(&params.handoff_id)?;
    Ok(HandoffRecoverResult {
        outcome,
        handoff,
        journal,
    })
}

fn handoff_mutation_result(
    metadata: &MetadataDb,
    handoff: HandoffRecord,
) -> Result<HandoffMutationResult> {
    let journal = metadata.list_handoff_journal(&handoff.handoff_id)?;
    Ok(HandoffMutationResult { handoff, journal })
}

fn require_handoff_project(
    metadata: &MetadataDb,
    expected_project: &str,
    handoff_id: &str,
) -> Result<HandoffRecord> {
    let handoff = metadata
        .get_handoff(handoff_id)?
        .ok_or_else(|| DevRelayError::Config(format!("unknown handoff {handoff_id}")))?;
    require_project(&handoff.project_id, expected_project, "remote handoff")?;
    Ok(handoff)
}

fn require_project(actual: &str, expected: &str, operation: &str) -> Result<()> {
    if actual != expected {
        return Err(DevRelayError::Config(format!(
            "{operation} project mismatch: expected {expected}, got {actual}"
        )));
    }
    Ok(())
}

fn require_actor(expected: Option<&str>, actor_device_id: &str, operation: &str) -> Result<()> {
    if expected != Some(actor_device_id) {
        return Err(DevRelayError::Config(format!(
            "{operation} actor mismatch: expected {}, got {actor_device_id}",
            expected.unwrap_or("<none>")
        )));
    }
    Ok(())
}

fn require_source_or_target_actor(
    handoff: &HandoffRecord,
    actor_device_id: &str,
    operation: &str,
) -> Result<()> {
    if actor_device_id != handoff.source_device_id && actor_device_id != handoff.target_device_id {
        return Err(DevRelayError::Config(format!(
            "{operation} actor mismatch: expected source {} or target {}, got {actor_device_id}",
            handoff.source_device_id, handoff.target_device_id
        )));
    }
    Ok(())
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
        HandoffRecoveryOutcome, HandoffState, LeaseRecord, LeaseState, ProjectRegistryEntry,
        SessionState, SnapshotMetadata, WorkspaceRegistryEntry,
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

    #[test]
    fn remote_sessions_snapshots_list_returns_redacted_summaries() {
        let temp = tempfile::tempdir().unwrap();
        let db = MetadataDb::open(temp.path().join("metadata.db")).unwrap();
        let default_session = db
            .ensure_default_session("project-a", "Demo", None)
            .unwrap();
        let fork_session = db
            .insert_session(
                "project-a",
                "Experiment",
                Some(&default_session.session_id),
                None,
                SessionState::Fork,
            )
            .unwrap();
        insert_snapshot(
            &db,
            "s1_000000000000000000000001",
            &default_session.session_id,
            1,
            false,
            Some("default checkpoint"),
            1_700_000_000,
        );
        insert_snapshot(
            &db,
            "s1_000000000000000000000002",
            &fork_session.session_id,
            2,
            true,
            Some("fork checkpoint"),
            1_700_000_100,
        );

        let result = remote_sessions_snapshots_list(
            &db,
            RemoteSessionsSnapshotsListParams {
                project: "project-a".to_string(),
                session_id: Some(fork_session.session_id.clone()),
                limit: Some(10),
            },
        )
        .unwrap();
        let encoded = serde_json::to_string(&result).unwrap();

        assert_eq!(result.snapshots.len(), 1);
        assert_eq!(
            result.snapshots[0].snapshot_id,
            "s1_000000000000000000000002"
        );
        assert_eq!(
            result.snapshots[0].session_id.as_deref(),
            Some(fork_session.session_id.as_str())
        );
        assert!(result.snapshots[0].pinned);
        assert!(!encoded.contains("metadata"));
        assert!(!encoded.contains("head_oid"));
        assert!(!encoded.contains("index_tree_oid"));
    }

    #[test]
    fn remote_sessions_snapshots_list_limits_newest_first() {
        let temp = tempfile::tempdir().unwrap();
        let db = MetadataDb::open(temp.path().join("metadata.db")).unwrap();
        let session = db
            .ensure_default_session("project-a", "Demo", None)
            .unwrap();
        for sequence in 1..=3 {
            insert_snapshot(
                &db,
                &format!("s1_00000000000000000000000{sequence}"),
                &session.session_id,
                sequence,
                false,
                None,
                1_700_000_000 + sequence as u64,
            );
        }

        let result = remote_sessions_snapshots_list(
            &db,
            RemoteSessionsSnapshotsListParams {
                project: "project-a".to_string(),
                session_id: None,
                limit: Some(2),
            },
        )
        .unwrap();

        assert_eq!(
            result
                .snapshots
                .iter()
                .map(|snapshot| snapshot.sequence_number)
                .collect::<Vec<_>>(),
            vec![3, 2]
        );
    }

    #[test]
    fn remote_handoff_handlers_complete_role_gated_flow() {
        let (_temp, mut db, lease) = handoff_db();
        let begin = remote_handoff_begin(
            &mut db,
            "device-a",
            HandoffBeginParams {
                project: "project123".to_string(),
                lease_id: lease.lease_id.clone(),
                target_device_id: "device-b".to_string(),
                source_generation: "gen-1".to_string(),
                ttl_seconds: Some(60),
            },
        )
        .unwrap();
        assert_eq!(begin.handoff.state, HandoffState::TargetPrepare);
        assert_eq!(begin.journal.len(), 2);

        let wrong_actor = remote_handoff_target_verify(
            &mut db,
            "device-a",
            HandoffIdParams {
                project: "project123".to_string(),
                handoff_id: begin.handoff.handoff_id.clone(),
            },
        )
        .unwrap_err();
        assert!(wrong_actor.to_string().contains("actor mismatch"));

        let verified = remote_handoff_target_verify(
            &mut db,
            "device-b",
            HandoffIdParams {
                project: "project123".to_string(),
                handoff_id: begin.handoff.handoff_id.clone(),
            },
        )
        .unwrap();
        assert_eq!(verified.handoff.state, HandoffState::TargetVerified);

        let ready = remote_handoff_source_ready(
            &mut db,
            "device-a",
            HandoffIdParams {
                project: "project123".to_string(),
                handoff_id: begin.handoff.handoff_id.clone(),
            },
        )
        .unwrap();
        assert_eq!(ready.handoff.state, HandoffState::SourceReady);

        let committed = remote_handoff_commit(
            &mut db,
            "device-a",
            HandoffCommitParams {
                project: "project123".to_string(),
                handoff_id: begin.handoff.handoff_id.clone(),
                observed_source_generation: "gen-1".to_string(),
            },
            begin.handoff.expires_at_unix_seconds - 1,
        )
        .unwrap();
        assert_eq!(committed.handoff.state, HandoffState::Committed);

        let updated_lease = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(updated_lease.holder_device_id.as_deref(), Some("device-b"));
        let listed = remote_handoffs_list(
            &db,
            HandoffsListParams {
                project: Some("project123".to_string()),
                include_journal: true,
            },
        )
        .unwrap();
        assert_eq!(listed.handoffs.len(), 1);
        assert!(listed.handoffs[0].journal.len() >= 4);
    }

    #[test]
    fn remote_handoff_begin_rejects_project_mismatch_before_mutation() {
        let (_temp, mut db, lease) = handoff_db();

        let err = remote_handoff_begin(
            &mut db,
            "device-a",
            HandoffBeginParams {
                project: "other-project".to_string(),
                lease_id: lease.lease_id,
                target_device_id: "device-b".to_string(),
                source_generation: "gen-1".to_string(),
                ttl_seconds: None,
            },
        )
        .unwrap_err();

        assert!(err.to_string().contains("project mismatch"));
        assert!(db.list_handoffs(None).unwrap().is_empty());
    }

    #[test]
    fn remote_handoff_abort_and_recover_are_actor_gated() {
        let (_temp, mut db, lease) = handoff_db();
        let aborting = remote_handoff_begin(
            &mut db,
            "device-a",
            HandoffBeginParams {
                project: "project123".to_string(),
                lease_id: lease.lease_id.clone(),
                target_device_id: "device-b".to_string(),
                source_generation: "gen-1".to_string(),
                ttl_seconds: Some(60),
            },
        )
        .unwrap();
        let wrong_abort = remote_handoff_abort(
            &mut db,
            "device-c",
            HandoffIdParams {
                project: "project123".to_string(),
                handoff_id: aborting.handoff.handoff_id.clone(),
            },
        )
        .unwrap_err();
        assert!(wrong_abort.to_string().contains("actor mismatch"));

        let aborted = remote_handoff_abort(
            &mut db,
            "device-b",
            HandoffIdParams {
                project: "project123".to_string(),
                handoff_id: aborting.handoff.handoff_id,
            },
        )
        .unwrap();
        assert_eq!(aborted.handoff.state, HandoffState::Aborted);

        let (_temp, mut db, lease) = handoff_db();
        let recovering = remote_handoff_begin(
            &mut db,
            "device-a",
            HandoffBeginParams {
                project: "project123".to_string(),
                lease_id: lease.lease_id.clone(),
                target_device_id: "device-b".to_string(),
                source_generation: "gen-1".to_string(),
                ttl_seconds: Some(60),
            },
        )
        .unwrap();
        let recovering_handoff_id = recovering.handoff.handoff_id.clone();
        let recovering_expires_at = recovering.handoff.expires_at_unix_seconds;
        remote_handoff_target_verify(
            &mut db,
            "device-b",
            HandoffIdParams {
                project: "project123".to_string(),
                handoff_id: recovering_handoff_id.clone(),
            },
        )
        .unwrap();
        remote_handoff_source_ready(
            &mut db,
            "device-a",
            HandoffIdParams {
                project: "project123".to_string(),
                handoff_id: recovering_handoff_id.clone(),
            },
        )
        .unwrap();

        let recovered = remote_handoff_recover(
            &mut db,
            "device-b",
            HandoffRecoverParams {
                project: "project123".to_string(),
                handoff_id: recovering_handoff_id,
                observed_source_generation: "gen-1".to_string(),
            },
            recovering_expires_at - 1,
        )
        .unwrap();
        assert_eq!(recovered.outcome, HandoffRecoveryOutcome::Committed);
        assert_eq!(recovered.handoff.state, HandoffState::Committed);
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

    fn insert_snapshot(
        db: &MetadataDb,
        snapshot_id: &str,
        session_id: &str,
        sequence_number: i64,
        pinned: bool,
        label: Option<&str>,
        created_at_unix_seconds: u64,
    ) {
        let mut metadata: SnapshotMetadata =
            serde_json::from_str(include_str!("../tests/fixtures/snapshot_metadata_v1.json"))
                .unwrap();
        metadata.snapshot_id = snapshot_id.to_string();
        metadata.project_id = "project-a".to_string();
        metadata.project_name = "Demo".to_string();
        metadata.session_id = Some(session_id.to_string());
        metadata.created_at_unix_seconds = created_at_unix_seconds;
        let metadata_json = serde_json::to_string(&metadata).unwrap();
        db.connection()
            .execute(
                r#"
INSERT INTO snapshots (
    snapshot_id,
    project_id,
    session_id,
    parent_snapshot_id,
    sequence_number,
    pinned,
    label,
    metadata_json,
    created_at_unix_seconds
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
"#,
                (
                    snapshot_id,
                    "project-a",
                    session_id,
                    Option::<&str>::None,
                    sequence_number,
                    pinned,
                    label,
                    metadata_json.as_str(),
                    created_at_unix_seconds as i64,
                ),
            )
            .unwrap();
    }

    fn handoff_db() -> (tempfile::TempDir, MetadataDb, LeaseRecord) {
        let temp = tempfile::tempdir().unwrap();
        let db = MetadataDb::open(temp.path().join("metadata.db")).unwrap();
        let session = db
            .ensure_default_session("project123", "Demo Project", None)
            .unwrap();
        let lease = LeaseRecord {
            lease_id: "lease-1".to_string(),
            project_id: "project123".to_string(),
            session_id: session.session_id,
            state: LeaseState::Active,
            epoch: 7,
            holder_device_id: Some("device-a".to_string()),
            latest_snapshot_id: None,
            handoff_id: None,
        };
        db.upsert_lease(&lease).unwrap();
        (temp, db, lease)
    }
}
