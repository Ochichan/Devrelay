//! Core DevRelay state capture and apply primitives.
//!
//! The public M0 surface is intentionally small: manifest parsing, Git-backed
//! status collection, untracked-path classification, and local snapshot
//! creation/apply helpers. Higher-level registry, agent, and UI state should be
//! built above this crate rather than inferred independently.
//!
//! Parse a manifest:
//!
//! ```
//! use devrelay_core::{Manifest, UntrackedPolicy};
//!
//! let manifest = Manifest::parse(r#"
//! schema = 1
//! project_id = "12345678"
//! name = "demo"
//!
//! [workspace]
//! untracked = "safe"
//! portable_paths = "strict"
//! "#)?;
//!
//! assert_eq!(manifest.workspace.untracked, UntrackedPolicy::Safe);
//! # Ok::<(), devrelay_core::DevRelayError>(())
//! ```
//!
//! Collect Git status:
//!
//! ```no_run
//! use devrelay_core::GitRepo;
//!
//! let repo = GitRepo::new(".");
//! let status = repo.status()?;
//! println!("{}", status.short_summary());
//! # Ok::<(), devrelay_core::DevRelayError>(())
//! ```

mod config;
mod error;
mod events;
mod git;
mod home;
mod ipc;
pub mod manifest;
mod policy;
mod rpc;
mod snapshot;
mod snapshot_schema;
mod snapshot_store;
mod storage;

pub use config::{
    AnchorMode, EditorPreference, LocalConfig, ProjectRegistryEntry, ProjectRegistryIndex,
    RedactedLocalConfig, ResourceProfile, WORKSPACE_ID_PREFIX, WorkspaceRegistryEntry,
    WorkspaceState, migrate_local_config, workspace_id_for,
};
pub use error::{DevRelayError, ErrorInfo, Result};
pub use events::{
    EVENT_SCHEMA_VERSION, EventEnvelope, EventReplayCursor, EventSequence, EventSequencer,
    EventTimestampMillis, EventType,
};
pub use git::{GitRepo, GitStatus, StatusCounts, StatusEntry, StatusEntryKind, StatusSummary};
pub use home::DevRelayHome;
pub use ipc::{IpcConnection, IpcLimits, IpcTransport, PeerCredentials};
#[cfg(unix)]
pub use ipc::{UnixIpcConnection, UnixIpcListener};
pub use manifest::{
    DirtyTargetPolicy, EnvironmentConfig, EnvironmentKind, EnvironmentProfile, HandoffConfig,
    Manifest, PatternConfig, PortablePathsPolicy, RestoreTerminals, SecretConfig, SecretMode,
    SyncConfig, SyncMode, TaskCacheMode, TaskConfig, TaskSandbox, UntrackedPolicy, WorkspaceConfig,
};
pub use policy::{ClassifiedPath, PathDecision, classification_reason, classify_untracked_paths};
#[cfg(unix)]
pub use rpc::AgentRpcClient;
pub use rpc::{
    ApplySnapshotParams, ApplySnapshotResult, CheckpointCreateParams, CheckpointCreateResult,
    DiagnosticsExportParams, DiagnosticsExportResult, METHOD_AGENT_HEALTH, METHOD_APPLY_SNAPSHOT,
    METHOD_CHECKPOINT_CREATE, METHOD_DIAGNOSTICS_EXPORT, METHOD_PROJECTS_ADD, METHOD_PROJECTS_LIST,
    METHOD_PROJECTS_REMOVE, METHOD_PROJECTS_SHOW, METHOD_RECOVER_LIST, METHOD_RECOVER_OPEN,
    METHOD_RECOVER_SHOW, METHOD_RPC_NEGOTIATE, METHOD_SNAPSHOTS_LIST, METHOD_STATUS_GET,
    ProjectResult, ProjectsAddParams, ProjectsListResult, ProjectsRemoveParams, ProjectsShowParams,
    RPC_JSONRPC_VERSION, RPC_PROTOCOL_VERSION, RecoverListParams, RecoverListResult,
    RecoverOpenParams, RecoverOpenResult, RecoverShowParams, RecoverShowResult, RpcError, RpcId,
    RpcRequest, RpcResponse, RpcVersionNegotiationParams, RpcVersionNegotiationResult,
    SnapshotsListParams, SnapshotsListResult, StatusGetParams, StatusGetResult,
};
pub use snapshot::{
    ApplyPlan, VerificationDetails, apply_snapshot, create_snapshot, plan_apply_snapshot,
    read_snapshot_file, verify_snapshot, write_snapshot_file,
};
pub use snapshot_schema::SnapshotMetadata;
pub use snapshot_store::{SnapshotStore, StoredSnapshot};
pub use storage::MetadataDb;
