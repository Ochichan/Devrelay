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
mod environment;
mod environment_doctor;
mod error;
mod events;
mod fs_safety;
mod git;
mod git_doctor;
mod handoff;
mod home;
mod hydration;
mod identity;
mod ipc;
mod lease;
mod lfs;
mod line_ending_doctor;
mod logging;
pub mod manifest;
mod nix_delegation;
mod operation_capsule;
mod pairing;
mod path_doctor;
mod platform;
mod policy;
mod retention;
mod route_selection;
mod rpc;
mod scheduler_constraints;
mod scheduler_score;
mod secret_provider;
mod service;
mod session;
mod sidecar;
mod snapshot;
mod snapshot_schema;
mod snapshot_store;
mod snapshot_upload;
mod sparse;
mod storage;
mod submodule;
mod task_artifacts;
mod task_cache;
mod task_logs;
mod task_model;
mod task_runner_execution;
mod task_runner_workspace;
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
    DEVRELAY_REF_NAMESPACE, DEVRELAY_SNAPSHOT_REF_NAMESPACE, GitDataPlaneAuthorization,
    GitDataPlaneAuthorizationRequest, GitDataPlaneImplementationStrategy, GitDataPlaneOperation,
    GitDataPlanePolicy, GitDataPlaneRefSpec, GitDataPlaneServePlan, GitObjectInspection,
    GitRepositorySize, authorize_git_data_plane_project, ensure_git_object_available,
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
pub use environment::{
    ContainerEngine, DevContainerAdapterReport, DevContainerHealthState, DevContainerHealthcheck,
    DevContainerImagePrepare, DevContainerMountPlan, DevContainerPrepareState, EnvironmentCommand,
    EnvironmentCommandOutput, EnvironmentCommandRunner, EnvironmentProfileSelection,
    EnvironmentSelectionContext, NativeBootstrapAdapterReport, NativeBootstrapRun,
    NativeBootstrapShell, NativeBootstrapState, NativeHealthState, NativeHealthcheck,
    NixAdapterReport, NixCacheWarmth, NixDevelopHealthcheck, NixFlakeFiles, NixHealthState,
    NixPlaceholderPlan, SystemEnvironmentCommandRunner, classify_native_command,
    compute_devcontainer_fingerprint, compute_nix_flake_fingerprint, detect_container_engine,
    detect_devcontainer_config, detect_nix_availability, detect_nix_flake_files,
    devcontainer_mount_plan, environment_profile_command_scope, estimate_nix_cache_warmth,
    inspect_devcontainer_environment, inspect_native_environment, inspect_nix_environment,
    nix_lan_binary_cache_plan, nix_store_prefetch_plan, prepare_devcontainer_image,
    profile_targets_platform, run_devcontainer_healthcheck, run_native_bootstrap,
    run_native_healthcheck, run_nix_develop_healthcheck, select_environment_profile,
};
pub use environment_doctor::{
    EnvironmentDoctorIssue, EnvironmentDoctorIssueCode, EnvironmentDoctorOptions,
    EnvironmentDoctorReport, run_environment_doctor,
};
pub use error::{DevRelayError, ErrorInfo, Result};
pub use events::{
    EVENT_SCHEMA_VERSION, EnvironmentProgressEvent, EventEnvelope, EventGapDetector,
    EventReplayCursor, EventSequence, EventSequenceGap, EventSequencer, EventStreamMessage,
    EventTimestampMillis, EventType, HandoffStateChangedEvent, ProtectionStatus,
    ProtectionStatusEvent, QuotaWarningEvent, SecurityBlockedEvent, SessionDivergedEvent,
    SnapshotApplyStartedEvent, SnapshotApplyVerifiedEvent, SnapshotLocalCreatedEvent,
    TypedEventPayload, WorkspaceStateChangedEvent,
};
pub use git::{
    GitRepo, GitStatus, StatusCounts, StatusEntry, StatusEntryKind, StatusSummary,
    parse_status_porcelain_v2,
};
pub use git_doctor::{
    GitPerformanceDoctorReport, GitPerformanceFix, GitPerformanceRecommendation, GitVersion,
    run_git_performance_doctor,
};
pub use handoff::{
    HANDOFF_ID_PREFIX, HandoffJournalPhase, HandoffJournalRecord, HandoffRecord,
    HandoffRecoveryOutcome, HandoffState, generate_handoff_id,
};
pub use home::{AnchorLayout, DevRelayHome};
pub use hydration::{
    HydrationProgress, HydrationState, HydrationStateMachine, HydrationStateRecord,
    HydrationTransition, load_hydration_state, save_hydration_state,
};
pub use identity::{
    DeviceCertificate, DevicePublicIdentity, DeviceRevocationRecord, FABRIC_ID_PREFIX,
    FabricIdentityBundle, FabricIdentityStore, FabricRootIdentity, RecoveryExportStatus,
};
pub use ipc::{IpcConnection, IpcLimits, IpcTransport, PeerCredentials};
#[cfg(unix)]
pub use ipc::{UnixIpcConnection, UnixIpcListener};
pub use lease::{LeaseRecord, LeaseState};
pub use lfs::{
    LFS_LOCAL_OBJECT_SIDECAR_CLASSIFICATION, LfsLocalOnlyObject, LfsMissingObject, LfsObjectReport,
    LfsPointer, capture_local_only_lfs_objects, ensure_lfs_objects_available,
    ensure_lfs_report_objects_available, ensure_snapshot_lfs_objects_available,
    ensure_snapshot_lfs_objects_available_or_sidecars, inspect_lfs_objects,
    inspect_lfs_objects_with_upstream, inspect_snapshot_lfs_objects,
    snapshot_has_lfs_object_sidecar,
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
pub use nix_delegation::{
    NixDelegationDecision, NixDelegationOptions, NixDelegationPlan, NixLanBinaryCachePublishPlan,
    NixLanBinaryCacheTarget, NixRemoteBuilderLogPlan, NixTemporaryBuilderSet, plan_nix_delegation,
    write_nix_temporary_builder_set,
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
pub use policy::{
    ClassifiedPath, PathDecision, classification_reason, classify_untracked_paths,
    normalize_workspace_relative_path,
};
pub use retention::{
    HandoffSnapshotProtection, PruningDecision, PruningDecisionAction, PruningPlan,
    PruningPlanInput, PruningPlanWarning, PruningPlanWarningCode, PruningReason, PruningScope,
    RetentionKeepReason, RetentionPolicy, SnapshotRetentionEntry, plan_snapshot_pruning,
};
pub use route_selection::{
    SnapshotRouteDecision, SnapshotRouteMeasurementInput, SnapshotRouteMeasurements,
    SnapshotRouteMetrics, SnapshotRoutePolicy, SnapshotTransferRoute, measure_snapshot_route,
    select_snapshot_route, select_snapshot_route_after_failure,
};
#[cfg(unix)]
pub use rpc::AgentRpcClient;
pub use rpc::{
    ActivityListParams, ActivityListResult, ApplySnapshotParams, ApplySnapshotResult,
    CheckpointCreateParams, CheckpointCreateResult, DevicesListResult, DiagnosticsExportParams,
    DiagnosticsExportResult, EditorContextLatestParams, EditorContextLatestResult,
    EditorContextSnapshot, EditorContextUpdateParams, EditorContextUpdateResult, EditorEventKind,
    EditorEventRecordParams, EditorEventRecordResult, EditorRestoreAckParams,
    EditorRestoreAckResult, EventsSubscribeParams, EventsSubscribeResult, HandoffBeginParams,
    HandoffCommitParams, HandoffIdParams, HandoffMutationResult, HandoffRecoverParams,
    HandoffRecoverResult, HandoffStatus, HandoffsListParams, HandoffsListResult, LeasesListParams,
    LeasesListResult, METHOD_ACTIVITY_LIST, METHOD_AGENT_HEALTH, METHOD_APPLY_SNAPSHOT,
    METHOD_CHECKPOINT_CREATE, METHOD_DEVICES_LIST, METHOD_DIAGNOSTICS_EXPORT,
    METHOD_EDITOR_CONTEXT_LATEST, METHOD_EDITOR_CONTEXT_UPDATE, METHOD_EDITOR_EVENT_RECORD,
    METHOD_EDITOR_RESTORE_ACK, METHOD_EVENTS_SUBSCRIBE, METHOD_HANDOFF_ABORT, METHOD_HANDOFF_BEGIN,
    METHOD_HANDOFF_COMMIT, METHOD_HANDOFF_RECOVER, METHOD_HANDOFF_SOURCE_READY,
    METHOD_HANDOFF_TARGET_VERIFY, METHOD_HANDOFFS_LIST, METHOD_LEASES_LIST, METHOD_PROJECTS_ADD,
    METHOD_PROJECTS_LIST, METHOD_PROJECTS_REMOVE, METHOD_PROJECTS_SHOW, METHOD_RECOVER_LIST,
    METHOD_RECOVER_OPEN, METHOD_RECOVER_SHOW, METHOD_RPC_NEGOTIATE, METHOD_RUNS_LIST,
    METHOD_SETTINGS_GET, METHOD_SETTINGS_UPDATE, METHOD_SNAPSHOTS_LIST, METHOD_STATUS_GET,
    ProjectResult, ProjectsAddParams, ProjectsListResult, ProjectsRemoveParams, ProjectsShowParams,
    RPC_JSONRPC_VERSION, RPC_PROTOCOL_VERSION, RecoverListParams, RecoverListResult,
    RecoverOpenParams, RecoverOpenResult, RecoverShowParams, RecoverShowResult, RpcError, RpcId,
    RpcRequest, RpcResponse, RpcVersionNegotiationParams, RpcVersionNegotiationResult,
    RunsListParams, RunsListResult, SettingsGetResult, SettingsUpdateParams, SettingsUpdateResult,
    SnapshotsListParams, SnapshotsListResult, StatusGetParams, StatusGetResult,
};
pub use scheduler_constraints::{
    SchedulerConstraintDecision, SchedulerConstraintRejection, SchedulerDevicePolicy,
    SchedulerDeviceSnapshot, SchedulerDynamicResources, SchedulerNetworkRouteQuality,
    collect_local_scheduler_device, evaluate_scheduler_constraints, filter_scheduler_candidates,
};
pub use scheduler_score::{
    SchedulerScore, SchedulerScoreComponent, SchedulerScoreComponentKind,
    SchedulerScoreMeasurements, SchedulerScoreWeights, SchedulerTaskClass,
    SchedulerThermalPressure, infer_scheduler_task_class, scheduler_score_components,
    score_scheduler_candidate, score_scheduler_candidate_with_class,
};
pub use secret_provider::{
    RedactedSecretMaterializationReport, SecretFileMaterialization, SecretMaterializationReport,
    SecretProvider, SecretProviderCommandPlan, SecretProviderKind, SecretProviderLocalConfig,
    SecretProviderMapping, SecretProviderRequest, SecretValue, materialize_project_secrets,
    secret_hard_exclude_patterns,
};
pub use service::{
    LINUX_SYSTEMD_UNIT, MACOS_LAUNCH_AGENT_LABEL, ServiceTemplate, ServiceTemplateInput,
    ServiceTemplateKind, linux_systemd_user_template, macos_launch_agent_template,
};
pub use session::{
    SESSION_ID_PREFIX, SessionState, StoredSession, generate_session_id, unix_now_seconds,
};
pub use sidecar::{
    DEFAULT_SIDECAR_CHUNK_BYTES, capture_large_sidecars, ensure_sidecars_available,
    materialize_sidecars,
};
pub use snapshot::{
    ApplyPlan, SnapshotApplyFaultPoint, VerificationDetails, apply_snapshot,
    apply_snapshot_with_fault_injection, apply_snapshot_with_journal, apply_snapshot_with_sidecars,
    create_snapshot, create_snapshot_with_sidecars, create_snapshot_with_sidecars_and_lfs_upstream,
    plan_apply_snapshot, read_snapshot_file, verify_snapshot, write_snapshot_file,
};
pub use snapshot_schema::{SnapshotChildSnapshot, SnapshotMetadata, SnapshotSidecar};
pub use snapshot_store::{
    SnapshotCheckpointResult, SnapshotCheckpointWithChildren, SnapshotPruneResult, SnapshotStore,
    SnapshotStoreFaultPoint, StoredSnapshot,
};
pub use snapshot_upload::{
    PendingSnapshotUpload, PendingSnapshotUploadCleanup, SnapshotDataUpload,
    SnapshotDataUploadFaultPoint, cleanup_pending_snapshot_upload, finish_snapshot_upload,
    list_pending_snapshot_uploads, mark_snapshot_upload_pending,
    publish_snapshot_canonical_with_data,
};
pub use sparse::{
    BlobAvailabilityReport, PartialCloneState, SparseCheckoutReport, fetch_missing_blobs_on_demand,
    inspect_sparse_checkout,
};
pub use storage::{
    CanonicalPublishRequest, CanonicalPublishResult, CommandTrustDecision, CommandTrustEvaluation,
    CommandTrustRecord, CommandTrustStatus, HandoffCommitSnapshotPreflight,
    InactiveForkPublishRequest, InactiveForkPublishResult, MetadataDb, MetadataDbFaultPoint,
    PairingStartRequest, TaskRunRecord,
};
pub use submodule::{
    SUBMODULE_CHILD_SNAPSHOT_RELATIONSHIP, SubmoduleReport, SubmoduleState, SubmoduleStatus,
    dirty_submodule_child_manifest, dirty_submodule_child_project_id, inspect_submodules,
    inspect_submodules_with_depth, restore_clean_submodule_recorded_commit,
};
pub use task_artifacts::{
    TaskArtifactCaptureSummary, TaskArtifactEntry, TaskArtifactIndex, TaskArtifactPullResult,
    TaskArtifactRetentionResult, apply_task_artifact_retention, capture_task_artifacts,
    pull_task_artifact, read_task_artifact_index, task_artifact_index_path,
};
pub use task_cache::{
    TaskResultCacheEligibility, TaskResultCacheEntry, TaskResultCacheHit, TaskResultCacheKey,
    TaskResultCacheKeyParts, TaskResultCachePolicy, TaskResultCacheRestore,
    TaskResultCacheSidecarInput, TaskResultCacheStoreResult, lookup_task_result_cache,
    read_task_result_cache_entry, restore_task_result_cache_hit, store_task_result_cache,
    task_result_cache_eligibility, task_result_cache_entry_path, task_result_cache_key,
};
pub use task_logs::{
    TASK_LOG_TRUNCATION_MARKER, TaskLogRecord, TaskLogRetrieval, TaskLogStore, TaskLogStoreConfig,
    read_task_log_spool, task_log_spool_path,
};
pub use task_model::{
    TASK_RUN_ID_PREFIX, TaskDefinition, TaskExecutionSnapshot, TaskRunInput, TaskRunState,
    create_task_execution_snapshot, generate_task_run_id, task_command_definition_hash,
    task_definition, task_definitions_from_manifest, task_execution_snapshot_label,
};
pub use task_runner_execution::{
    NoopTaskExecutionLogSink, SystemTaskCommandRunner, TaskCommandOutput, TaskCommandRunner,
    TaskExecutionBackend, TaskExecutionLogEvent, TaskExecutionLogSink, TaskExecutionLogStream,
    TaskExecutionOptions, TaskExecutionResult, VecTaskExecutionLogSink,
    execute_task_in_runner_workspace, task_execution_backend,
};
pub use task_runner_workspace::{
    TaskRunnerEnvironmentState, TaskRunnerSecretPolicy, TaskRunnerSecretState,
    TaskRunnerSidecarState, TaskRunnerWorkspace, TaskRunnerWorkspaceCleanup,
    TaskRunnerWorkspaceOptions, TaskRunnerWorkspaceRetentionPolicy, cleanup_task_runner_workspace,
    prepare_task_runner_workspace, task_runner_workspace_path,
};
pub use transport_security::{
    AuthenticatedControlPlanePeer, CONTROL_ALPN_PROTOCOL, CONTROL_PROTOCOL_VERSION,
    ControlPlaneReplayCache, ControlPlaneRequestEnvelope, ControlPlaneTransportPolicy,
    ControlPlaneTransportSecurity, RustlsIdentity, ValidatedDeviceCertificate,
    build_rustls_client_config, build_rustls_server_config, negotiate_control_protocol_version,
    require_authenticated_control_channel, validate_control_request_envelope,
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
