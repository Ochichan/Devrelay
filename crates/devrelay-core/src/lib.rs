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

mod anchor_snapshot;
mod audit;
mod background_checkpoint;
mod cas;
mod config;
mod crash_journal;
mod data_plane;
mod debounce;
mod discovery;
mod error;
mod events;
mod fs_safety;
mod git;
mod git_doctor;
mod handoff;
mod home;
mod identity;
mod ipc;
mod lease;
mod lfs;
mod line_ending_doctor;
mod logging;
pub mod manifest;
mod operation_capsule;
mod pairing;
mod path_doctor;
mod platform;
mod policy;
mod retention;
mod route_selection;
mod rpc;
mod service;
mod session;
mod sidecar;
mod snapshot;
mod snapshot_schema;
mod snapshot_store;
mod sparse;
mod storage;
mod submodule;
mod transport_security;
mod watcher;
mod wsl_doctor;

pub use anchor_snapshot::{AnchorSnapshotMaintenanceReport, AnchorSnapshotRef, AnchorSnapshotRepo};
pub use audit::{
    AUDIT_SCHEMA_VERSION, AuditEventInput, AuditEventRecord, AuditEventType, AuditOutcome,
};
pub use background_checkpoint::{
    BackgroundCheckpointManager, BackgroundCheckpointOutcome, BackgroundCheckpointReport,
    BackgroundWorkspace, DEFAULT_BACKGROUND_FAILURE_NOTIFICATION_THRESHOLD,
    WorkspaceCheckpointState,
};
pub use cas::{
    CAS_HASH_PREFIX, CAS_SCHEMA_VERSION, CasChunkHash, CasChunkRecord, CasManifest,
    CasManifestChunk, CasReachabilityRoot, CasStore, CasUploadResult,
};
pub use config::{
    AgentRole, AnchorMode, DEVICE_ID_PREFIX, DeviceIdentity, EditorPreference, ForegroundLoad,
    LocalConfig, ProjectRegistryEntry, ProjectRegistryIndex, RedactedLocalConfig, ResourcePolicy,
    ResourcePolicyContext, ResourcePolicyLimits, ResourcePowerSource, ResourceProfile,
    WORKSPACE_ID_PREFIX, WorkspaceRegistryEntry, WorkspaceState, detect_resource_policy_context,
    generate_device_id, migrate_local_config, workspace_id_for,
};
pub use crash_journal::{
    CrashJournal, CrashJournalFaultPoint, CrashJournalOperationReplay, CrashJournalPhase,
    CrashJournalRecord, CrashJournalReplay,
};
pub use data_plane::{
    DEVRELAY_REF_NAMESPACE, DEVRELAY_SNAPSHOT_REF_NAMESPACE, GitDataPlanePolicy,
    GitDataPlaneRefSpec, GitObjectInspection, GitRepositorySize, ensure_git_object_available,
    inspect_git_object, inspect_git_repository_size, verify_git_repository_integrity,
};
pub use debounce::{
    AdaptiveDebouncer, BackgroundDebouncePolicy, DebounceDrain, DebounceFlushReason,
    DebouncedCheckpoint, DebouncedPublish,
};
pub use discovery::{
    DEVRELAY_ANCHOR_SERVICE_TYPE, DEVRELAY_DISCOVERY_PROTOCOL, DEVRELAY_PEER_SERVICE_TYPE,
    DISCOVERY_TXT_DEVICE_ID, DISCOVERY_TXT_FABRIC_HINT, DISCOVERY_TXT_PORT, DISCOVERY_TXT_PROTOCOL,
    DiscoveryAdvertisement, DiscoveryRole, DiscoveryService, build_discovery_advertisement,
    truncated_fabric_hint,
};
pub use error::{DevRelayError, ErrorInfo, Result};
pub use events::{
    EVENT_SCHEMA_VERSION, EventEnvelope, EventGapDetector, EventReplayCursor, EventSequence,
    EventSequenceGap, EventSequencer, EventStreamMessage, EventTimestampMillis, EventType,
    ProtectionStatus, ProtectionStatusEvent, QuotaWarningEvent, SecurityBlockedEvent,
    SessionDivergedEvent, SnapshotApplyStartedEvent, SnapshotApplyVerifiedEvent,
    SnapshotLocalCreatedEvent, TypedEventPayload, WorkspaceStateChangedEvent,
};
pub use git::{GitRepo, GitStatus, StatusCounts, StatusEntry, StatusEntryKind, StatusSummary};
pub use git_doctor::{
    GitPerformanceDoctorReport, GitPerformanceFix, GitPerformanceRecommendation, GitVersion,
    run_git_performance_doctor,
};
pub use handoff::{
    HANDOFF_ID_PREFIX, HandoffJournalPhase, HandoffJournalRecord, HandoffRecord,
    HandoffRecoveryOutcome, HandoffState, generate_handoff_id,
};
pub use home::{AnchorLayout, DevRelayHome};
pub use identity::{
    DeviceCertificate, DevicePublicIdentity, DeviceRevocationRecord, FABRIC_ID_PREFIX,
    FabricIdentityBundle, FabricIdentityStore, FabricRootIdentity, RecoveryExportStatus,
};
pub use ipc::{IpcConnection, IpcLimits, IpcTransport, PeerCredentials};
#[cfg(unix)]
pub use ipc::{UnixIpcConnection, UnixIpcListener};
pub use lease::{LeaseRecord, LeaseState};
pub use lfs::{
    LfsMissingObject, LfsObjectReport, LfsPointer, ensure_lfs_objects_available,
    inspect_lfs_objects,
};
pub use line_ending_doctor::{
    LineEndingDoctorReport, LineEndingHashMismatch, LineEndingWarning, LineEndingWarningCode,
    run_line_ending_doctor,
};
pub use logging::{
    LogRedactor, LogRotation, StructuredLogFile, StructuredLogFormat, StructuredLogLevel,
    StructuredLogRecord,
};
pub use manifest::{
    DirtyTargetPolicy, EnvironmentConfig, EnvironmentKind, EnvironmentProfile, HandoffConfig,
    Manifest, PatternConfig, PortablePathsPolicy, RestoreTerminals, SecretConfig, SecretMode,
    SecretScannerConfig, SyncConfig, SyncMode, TaskCacheMode, TaskConfig, TaskSandbox,
    UntrackedPolicy, WorkspaceConfig,
};
pub use operation_capsule::{
    ConflictWorktreeFile, GitOperationKind, GitOperationMetadata, GitOperationProgress,
    GitOperationStep, IndexStageEntry, OperationCapsule, REBASE_OPERATION_MIN_TARGET_GIT_VERSION,
    REBASE_OPERATION_RECONSTRUCTION_ENABLED, UnmergedIndexEntry, apply_unmerged_index_entries,
    capture_operation_capsule, restore_conflict_worktree_files,
};
pub use pairing::{
    PAIRING_ID_PREFIX, PairingEphemeralKey, PairingSession, PairingState,
    compute_handshake_transcript_hash, derive_short_authentication_string,
    generate_ephemeral_pairing_key, generate_pairing_id, validate_key_hex,
};
pub use path_doctor::{
    PathPortabilityDoctorReport, PathPortabilityIssue, PathPortabilityIssueCode,
    PathPortabilityPathSource, run_path_portability_doctor,
};
pub use platform::{
    PLATFORM_KEY_FORMAT, PlatformCapabilities, PlatformIdentity, WslIdentity,
    current_platform_architecture, current_platform_capabilities_json,
    current_platform_device_scope_key, current_platform_key, detect_platform_identity,
    platform_capabilities_for_key, platform_device_scope_key,
};
pub use policy::{ClassifiedPath, PathDecision, classification_reason, classify_untracked_paths};
pub use retention::{
    HandoffSnapshotProtection, PruningDecision, PruningDecisionAction, PruningPlan,
    PruningPlanInput, PruningPlanWarning, PruningPlanWarningCode, PruningReason, PruningScope,
    RetentionKeepReason, RetentionPolicy, SnapshotRetentionEntry, plan_snapshot_pruning,
};
pub use route_selection::{
    SnapshotRouteDecision, SnapshotRouteMeasurements, SnapshotRouteMetrics, SnapshotRoutePolicy,
    SnapshotTransferRoute, select_snapshot_route, select_snapshot_route_after_failure,
};
#[cfg(unix)]
pub use rpc::AgentRpcClient;
pub use rpc::{
    ApplySnapshotParams, ApplySnapshotResult, CheckpointCreateParams, CheckpointCreateResult,
    DiagnosticsExportParams, DiagnosticsExportResult, EventsSubscribeParams, EventsSubscribeResult,
    METHOD_AGENT_HEALTH, METHOD_APPLY_SNAPSHOT, METHOD_CHECKPOINT_CREATE,
    METHOD_DIAGNOSTICS_EXPORT, METHOD_EVENTS_SUBSCRIBE, METHOD_PROJECTS_ADD, METHOD_PROJECTS_LIST,
    METHOD_PROJECTS_REMOVE, METHOD_PROJECTS_SHOW, METHOD_RECOVER_LIST, METHOD_RECOVER_OPEN,
    METHOD_RECOVER_SHOW, METHOD_RPC_NEGOTIATE, METHOD_SNAPSHOTS_LIST, METHOD_STATUS_GET,
    ProjectResult, ProjectsAddParams, ProjectsListResult, ProjectsRemoveParams, ProjectsShowParams,
    RPC_JSONRPC_VERSION, RPC_PROTOCOL_VERSION, RecoverListParams, RecoverListResult,
    RecoverOpenParams, RecoverOpenResult, RecoverShowParams, RecoverShowResult, RpcError, RpcId,
    RpcRequest, RpcResponse, RpcVersionNegotiationParams, RpcVersionNegotiationResult,
    SnapshotsListParams, SnapshotsListResult, StatusGetParams, StatusGetResult,
};
pub use service::{
    LINUX_SYSTEMD_UNIT, MACOS_LAUNCH_AGENT_LABEL, ServiceTemplate, ServiceTemplateInput,
    ServiceTemplateKind, linux_systemd_user_template, macos_launch_agent_template,
};
pub use session::{
    SESSION_ID_PREFIX, SessionState, StoredSession, generate_session_id, unix_now_seconds,
};
pub use sidecar::{DEFAULT_SIDECAR_CHUNK_BYTES, capture_large_sidecars};
pub use snapshot::{
    ApplyPlan, SnapshotApplyFaultPoint, VerificationDetails, apply_snapshot,
    apply_snapshot_with_fault_injection, create_snapshot, create_snapshot_with_sidecars,
    plan_apply_snapshot, read_snapshot_file, verify_snapshot, write_snapshot_file,
};
pub use snapshot_schema::{SnapshotMetadata, SnapshotSidecar};
pub use snapshot_store::{
    SnapshotCheckpointResult, SnapshotPruneResult, SnapshotStore, SnapshotStoreFaultPoint,
    StoredSnapshot,
};
pub use sparse::{PartialCloneState, SparseCheckoutReport, inspect_sparse_checkout};
pub use storage::{
    CanonicalPublishRequest, CanonicalPublishResult, InactiveForkPublishRequest,
    InactiveForkPublishResult, MetadataDb, MetadataDbFaultPoint, PairingStartRequest,
};
pub use submodule::{
    SubmoduleReport, SubmoduleState, SubmoduleStatus, inspect_submodules,
    inspect_submodules_with_depth, restore_clean_submodule_recorded_commit,
};
pub use transport_security::{
    CONTROL_ALPN_PROTOCOL, CONTROL_PROTOCOL_VERSION, ControlPlaneReplayCache,
    ControlPlaneRequestEnvelope, ControlPlaneTransportPolicy, RustlsIdentity,
    ValidatedDeviceCertificate, build_rustls_client_config, build_rustls_server_config,
    negotiate_control_protocol_version, validate_control_request_envelope,
    validate_device_certificate,
};
#[cfg(target_os = "macos")]
pub use watcher::MacOsFilesystemWatcher;
pub use watcher::{
    CoalescedWorkspaceChange, FilesystemEventKind, FilesystemRawEvent, FilesystemWatchMessage,
    FilesystemWatchState, FilesystemWatcher, PollingFilesystemWatcher, WorkspaceChangeHint,
    WorkspaceWatch, default_filesystem_watcher,
};
pub use wsl_doctor::{
    WslFilesystemDoctorReport, WslFilesystemPathKind, WslFilesystemWarning,
    WslFilesystemWarningCode, run_wsl_filesystem_doctor,
};
