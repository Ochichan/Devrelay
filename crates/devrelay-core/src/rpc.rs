//! JSON-RPC envelope types for local agent IPC.
//!
//! DevRelay supports request/response JSON-RPC 2.0 over the local IPC
//! transport. Notifications are intentionally unsupported for M2 so every
//! request has a stable ID that can be echoed in success and error responses.

#[cfg(unix)]
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
#[cfg(unix)]
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    ApplyPlan, ClassifiedPath, ProjectRegistryEntry, StatusEntry, StatusSummary, StoredSnapshot,
    VerificationDetails, WorkspaceRegistryEntry,
};
#[cfg(unix)]
use crate::{DevRelayError, IpcConnection, IpcLimits, Result, UnixIpcConnection};
use crate::{EventReplayCursor, EventSequence};

pub const RPC_JSONRPC_VERSION: &str = "2.0";
pub const RPC_PROTOCOL_VERSION: u32 = 1;
pub const METHOD_RPC_NEGOTIATE: &str = "rpc.negotiate";
pub const METHOD_AGENT_HEALTH: &str = "agent.health";
pub const METHOD_STATUS_GET: &str = "status.get";
pub const METHOD_PROJECTS_ADD: &str = "projects.add";
pub const METHOD_PROJECTS_LIST: &str = "projects.list";
pub const METHOD_PROJECTS_SHOW: &str = "projects.show";
pub const METHOD_PROJECTS_REMOVE: &str = "projects.remove";
pub const METHOD_CHECKPOINT_CREATE: &str = "checkpoint.create";
pub const METHOD_SNAPSHOTS_LIST: &str = "snapshots.list";
pub const METHOD_APPLY_SNAPSHOT: &str = "apply.snapshot";
pub const METHOD_RECOVER_LIST: &str = "recover.list";
pub const METHOD_RECOVER_SHOW: &str = "recover.show";
pub const METHOD_RECOVER_OPEN: &str = "recover.open";
pub const METHOD_DIAGNOSTICS_EXPORT: &str = "diagnostics.export";
pub const METHOD_EVENTS_SUBSCRIBE: &str = "events.subscribe";

pub const RPC_PARSE_ERROR: i64 = -32700;
pub const RPC_INVALID_REQUEST: i64 = -32600;
pub const RPC_METHOD_NOT_FOUND: i64 = -32601;
pub const RPC_INVALID_PARAMS: i64 = -32602;
pub const RPC_INTERNAL_ERROR: i64 = -32603;
pub const RPC_VERSION_MISMATCH: i64 = -32001;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcId {
    String(String),
    Number(u64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    #[serde(default)]
    pub id: Option<RpcId>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

impl RpcRequest {
    pub fn parse(bytes: &[u8]) -> std::result::Result<Self, RpcError> {
        let request: Self =
            serde_json::from_slice(bytes).map_err(|err| RpcError::parse_error(err.to_string()))?;
        request.validate()?;
        Ok(request)
    }

    pub fn required_id(&self) -> std::result::Result<RpcId, RpcError> {
        self.id
            .clone()
            .ok_or_else(|| RpcError::invalid_request("request id is required"))
    }

    fn validate(&self) -> std::result::Result<(), RpcError> {
        if self.jsonrpc != RPC_JSONRPC_VERSION {
            return Err(RpcError::invalid_request(format!(
                "jsonrpc must be {RPC_JSONRPC_VERSION}"
            )));
        }
        if self.id.is_none() {
            return Err(RpcError::invalid_request(
                "request id is required; notifications are not supported",
            ));
        }
        if self.method.trim().is_empty() {
            return Err(RpcError::invalid_request("method must not be empty"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    pub id: Option<RpcId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl RpcResponse {
    pub fn success(id: RpcId, result: Value) -> Self {
        Self {
            jsonrpc: RPC_JSONRPC_VERSION.to_string(),
            id: Some(id),
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<RpcId>, error: RpcError) -> Self {
        Self {
            jsonrpc: RPC_JSONRPC_VERSION.to_string(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcError {
    pub fn parse_error(detail: impl Into<String>) -> Self {
        Self::with_detail(RPC_PARSE_ERROR, "Parse error", detail)
    }

    pub fn invalid_request(detail: impl Into<String>) -> Self {
        Self::with_detail(RPC_INVALID_REQUEST, "Invalid request", detail)
    }

    pub fn method_not_found(method: impl Into<String>) -> Self {
        Self::with_detail(
            RPC_METHOD_NOT_FOUND,
            "Method not found",
            format!("unknown RPC method {}", method.into()),
        )
    }

    pub fn invalid_params(detail: impl Into<String>) -> Self {
        Self::with_detail(RPC_INVALID_PARAMS, "Invalid params", detail)
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self::with_detail(RPC_INTERNAL_ERROR, "Internal error", detail)
    }

    pub fn version_mismatch(client_protocol_version: u32) -> Self {
        Self {
            code: RPC_VERSION_MISMATCH,
            message: "Protocol version mismatch".to_string(),
            data: Some(json!({
                "client_protocol_version": client_protocol_version,
                "server_protocol_version": RPC_PROTOCOL_VERSION,
            })),
        }
    }

    fn with_detail(code: i64, message: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            message: message.to_string(),
            data: Some(json!({ "detail": detail.into() })),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcVersionNegotiationParams {
    pub client_protocol_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcVersionNegotiationResult {
    pub protocol_version: u32,
    pub server_name: String,
    pub methods: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusGetParams {
    pub repo: PathBuf,
    pub manifest: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusGetResult {
    pub status: StatusSummary,
    pub entries: Vec<StatusEntry>,
    pub untracked: Vec<ClassifiedPath>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectsAddParams {
    pub path: PathBuf,
    pub manifest: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectsShowParams {
    pub id_or_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectsRemoveParams {
    pub id_or_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectResult {
    pub project: ProjectRegistryEntry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectsListResult {
    pub projects: Vec<ProjectRegistryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointCreateParams {
    pub repo: PathBuf,
    pub manifest: Option<PathBuf>,
    pub label: Option<String>,
    #[serde(default)]
    pub pin: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointCreateResult {
    pub checkpoint: StoredSnapshot,
    pub snapshot_repo: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotsListParams {
    pub project: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotsListResult {
    pub snapshots: Vec<StoredSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplySnapshotParams {
    pub repo: PathBuf,
    pub project: String,
    pub snapshot_id: String,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplySnapshotResult {
    pub snapshot: StoredSnapshot,
    pub plan: Option<ApplyPlan>,
    pub verification: Option<VerificationDetails>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverListParams {
    pub project: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverListResult {
    pub snapshots: Vec<StoredSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverShowParams {
    pub snapshot_id: String,
    pub project: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverShowResult {
    pub snapshot: StoredSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverOpenParams {
    pub snapshot_id: String,
    pub path: PathBuf,
    pub project: Option<String>,
    #[serde(default)]
    pub register: bool,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverOpenResult {
    pub recovered: StoredSnapshot,
    pub path: PathBuf,
    pub name: Option<String>,
    pub registered: Option<WorkspaceRegistryEntry>,
    pub verification: VerificationDetails,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticsExportParams {
    pub out: Option<PathBuf>,
    #[serde(default)]
    pub include_sensitive_paths: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticsExportResult {
    pub path: PathBuf,
    pub include_sensitive_paths: bool,
    pub source_code_included: bool,
    pub snapshot_objects_included: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventsSubscribeParams {
    #[serde(default)]
    pub cursor: EventReplayCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventsSubscribeResult {
    pub cursor: EventReplayCursor,
    pub replayed: usize,
    pub current_sequence: Option<EventSequence>,
}

#[cfg(unix)]
static NEXT_RPC_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(unix)]
#[derive(Debug, Clone)]
pub struct AgentRpcClient {
    socket_path: PathBuf,
    limits: IpcLimits,
}

#[cfg(unix)]
impl AgentRpcClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            limits: IpcLimits::default(),
        }
    }

    pub fn with_limits(socket_path: impl Into<PathBuf>, limits: IpcLimits) -> Self {
        Self {
            socket_path: socket_path.into(),
            limits,
        }
    }

    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    pub fn call<P, R>(&self, method: &str, params: P) -> Result<R>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let id = RpcId::String(format!(
            "client-{}-{}",
            std::process::id(),
            NEXT_RPC_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let request = RpcRequest {
            jsonrpc: RPC_JSONRPC_VERSION.to_string(),
            id: Some(id.clone()),
            method: method.to_string(),
            params: serde_json::to_value(params)?,
        };
        let mut connection = UnixIpcConnection::connect(&self.socket_path, self.limits)?;
        let request_bytes = serde_json::to_vec(&request)?;
        connection.write_message(&request_bytes, self.limits)?;
        let response_bytes = connection.read_message(self.limits)?;
        let response: RpcResponse = serde_json::from_slice(&response_bytes)?;

        if response.jsonrpc != RPC_JSONRPC_VERSION {
            return Err(DevRelayError::Ipc(format!(
                "agent returned unsupported jsonrpc version {}",
                response.jsonrpc
            )));
        }
        if response.id.as_ref() != Some(&id) {
            return Err(DevRelayError::Ipc("agent response ID mismatch".to_string()));
        }
        if let Some(error) = response.error {
            return Err(DevRelayError::Ipc(format!(
                "agent RPC error {}: {}",
                error.code, error.message
            )));
        }
        let result = response
            .result
            .ok_or_else(|| DevRelayError::Ipc("agent response missing result".to_string()))?;
        Ok(serde_json::from_value(result)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_request_and_requires_stable_id() {
        let request = RpcRequest::parse(
            br#"{"jsonrpc":"2.0","id":"abc","method":"agent.health","params":{}}"#,
        )
        .unwrap();

        assert_eq!(request.required_id().unwrap(), RpcId::String("abc".into()));
        assert_eq!(request.method, METHOD_AGENT_HEALTH);

        let err = RpcRequest::parse(br#"{"jsonrpc":"2.0","method":"agent.health"}"#).unwrap_err();
        assert_eq!(err.code, RPC_INVALID_REQUEST);
    }

    #[test]
    fn accepts_numeric_request_ids_and_echoes_them() {
        let request =
            RpcRequest::parse(br#"{"jsonrpc":"2.0","id":42,"method":"agent.health"}"#).unwrap();
        let response = RpcResponse::success(request.required_id().unwrap(), json!({"status":"ok"}));
        let encoded = serde_json::to_value(response).unwrap();

        assert_eq!(encoded["id"], 42);
        assert_eq!(encoded["result"]["status"], "ok");
        assert!(encoded.get("error").is_none());
    }

    #[test]
    fn serializes_error_envelope_with_null_id_for_invalid_request() {
        let response = RpcResponse::error(None, RpcError::invalid_request("bad envelope"));
        let encoded = serde_json::to_value(response).unwrap();

        assert_eq!(encoded["jsonrpc"], RPC_JSONRPC_VERSION);
        assert!(encoded["id"].is_null());
        assert_eq!(encoded["error"]["code"], RPC_INVALID_REQUEST);
        assert!(encoded.get("result").is_none());
    }

    #[test]
    fn rejects_wrong_jsonrpc_version_and_empty_method() {
        let wrong_version =
            RpcRequest::parse(br#"{"jsonrpc":"1.0","id":"a","method":"agent.health"}"#)
                .unwrap_err();
        assert_eq!(wrong_version.code, RPC_INVALID_REQUEST);

        let empty_method =
            RpcRequest::parse(br#"{"jsonrpc":"2.0","id":"a","method":"  "}"#).unwrap_err();
        assert_eq!(empty_method.code, RPC_INVALID_REQUEST);
    }

    #[test]
    fn version_mismatch_error_carries_client_and_server_versions() {
        let error = RpcError::version_mismatch(999);

        assert_eq!(error.code, RPC_VERSION_MISMATCH);
        assert_eq!(error.data.as_ref().unwrap()["client_protocol_version"], 999);
        assert_eq!(
            error.data.as_ref().unwrap()["server_protocol_version"],
            RPC_PROTOCOL_VERSION
        );
    }

    #[test]
    fn status_get_params_use_path_strings() {
        let params: StatusGetParams = serde_json::from_value(json!({
            "repo": "/tmp/repo",
            "manifest": "/tmp/repo/devrelay.toml"
        }))
        .unwrap();

        assert_eq!(params.repo, PathBuf::from("/tmp/repo"));
        assert_eq!(
            params.manifest.as_deref(),
            Some(std::path::Path::new("/tmp/repo/devrelay.toml"))
        );
    }

    #[test]
    fn project_registry_params_use_stable_field_names() {
        let add: ProjectsAddParams = serde_json::from_value(json!({
            "path": "/tmp/repo",
            "manifest": "/tmp/repo/devrelay.toml"
        }))
        .unwrap();
        assert_eq!(add.path, PathBuf::from("/tmp/repo"));

        let show: ProjectsShowParams = serde_json::from_value(json!({
            "id_or_name": "demo"
        }))
        .unwrap();
        assert_eq!(show.id_or_name, "demo");

        let remove: ProjectsRemoveParams = serde_json::from_value(json!({
            "id_or_name": "demo"
        }))
        .unwrap();
        assert_eq!(remove.id_or_name, "demo");
    }

    #[test]
    fn checkpoint_create_defaults_pin_to_false() {
        let params: CheckpointCreateParams = serde_json::from_value(json!({
            "repo": "/tmp/repo",
            "manifest": "/tmp/repo/devrelay.toml",
            "label": "manual"
        }))
        .unwrap();

        assert!(!params.pin);
        assert_eq!(params.label.as_deref(), Some("manual"));
    }

    #[test]
    fn apply_snapshot_defaults_dry_run_to_false() {
        let params: ApplySnapshotParams = serde_json::from_value(json!({
            "repo": "/tmp/target",
            "project": "12345678",
            "snapshot_id": "snap_abc"
        }))
        .unwrap();

        assert!(!params.dry_run);
        assert_eq!(params.project, "12345678");
    }

    #[test]
    fn recover_open_defaults_register_to_false() {
        let list: RecoverListParams = serde_json::from_value(json!({
            "project": null
        }))
        .unwrap();
        assert_eq!(list.project, None);

        let show: RecoverShowParams = serde_json::from_value(json!({
            "snapshot_id": "snap_abc",
            "project": "12345678"
        }))
        .unwrap();
        assert_eq!(show.snapshot_id, "snap_abc");
        assert_eq!(show.project.as_deref(), Some("12345678"));

        let params: RecoverOpenParams = serde_json::from_value(json!({
            "snapshot_id": "snap_abc",
            "path": "/tmp/recovery"
        }))
        .unwrap();

        assert!(!params.register);
        assert_eq!(params.project, None);
    }

    #[test]
    fn diagnostics_export_defaults_to_redacted_paths() {
        let params: DiagnosticsExportParams = serde_json::from_value(json!({})).unwrap();

        assert_eq!(params.out, None);
        assert!(!params.include_sensitive_paths);
    }

    #[test]
    fn events_subscribe_defaults_to_start_cursor() {
        let params: EventsSubscribeParams = serde_json::from_value(json!({})).unwrap();
        assert_eq!(params.cursor, EventReplayCursor::from_start());

        let params: EventsSubscribeParams = serde_json::from_value(json!({
            "cursor": { "after_sequence": 4 }
        }))
        .unwrap();
        assert_eq!(
            params.cursor.after_sequence,
            Some(EventSequence::new(4).unwrap())
        );

        let result = serde_json::to_value(EventsSubscribeResult {
            cursor: EventReplayCursor::after(EventSequence::new(4).unwrap()),
            replayed: 2,
            current_sequence: Some(EventSequence::new(6).unwrap()),
        })
        .unwrap();
        assert_eq!(result["cursor"]["after_sequence"], 4);
        assert_eq!(result["replayed"], 2);
        assert_eq!(result["current_sequence"], 6);
    }

    #[cfg(unix)]
    #[test]
    fn agent_rpc_client_round_trips_success_response() {
        use crate::{IpcConnection, IpcTransport, UnixIpcListener};
        use std::thread;

        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("agent.sock");
        let listener = UnixIpcListener::bind(&socket).unwrap();
        let handle = thread::spawn(move || {
            let mut connection = listener.accept().unwrap();
            let request_bytes = connection.read_message(IpcLimits::default()).unwrap();
            let request = RpcRequest::parse(&request_bytes).unwrap();
            assert_eq!(request.method, METHOD_AGENT_HEALTH);
            let response =
                RpcResponse::success(request.required_id().unwrap(), json!({ "status": "ok" }));
            connection
                .write_message(
                    &serde_json::to_vec(&response).unwrap(),
                    IpcLimits::default(),
                )
                .unwrap();
        });

        let client = AgentRpcClient::new(&socket);
        let response: serde_json::Value = client
            .call(METHOD_AGENT_HEALTH, json!({ "probe": true }))
            .unwrap();

        assert_eq!(response["status"], "ok");
        handle.join().unwrap();
    }
}
