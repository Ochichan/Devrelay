use devrelay_core::{
    ActivityListParams, ActivityListResult, AgentRpcClient, ApplySnapshotParams,
    ApplySnapshotResult, CheckpointCreateParams, CheckpointCreateResult, DevRelayHome,
    DevicesListResult, DiagnosticsExportParams, DiagnosticsExportResult, EventReplayCursor,
    EventStreamMessage, EventsSubscribeParams, EventsSubscribeResult, HandoffBeginParams,
    HandoffCommitParams, HandoffIdParams, HandoffMutationResult, HandoffState, HandoffStatus,
    HandoffsListParams, HandoffsListResult, IpcConnection, IpcLimits, LeaseRecord, LeaseState,
    LeasesListParams, LeasesListResult, METHOD_ACTIVITY_LIST, METHOD_AGENT_HEALTH,
    METHOD_APPLY_SNAPSHOT, METHOD_CHECKPOINT_CREATE, METHOD_DEVICES_LIST,
    METHOD_DIAGNOSTICS_EXPORT, METHOD_EVENTS_SUBSCRIBE, METHOD_HANDOFF_ABORT, METHOD_HANDOFF_BEGIN,
    METHOD_HANDOFF_COMMIT, METHOD_HANDOFF_SOURCE_READY, METHOD_HANDOFF_TARGET_VERIFY,
    METHOD_HANDOFFS_LIST, METHOD_LEASES_LIST, METHOD_PROJECTS_ADD, METHOD_PROJECTS_LIST,
    METHOD_RECOVER_OPEN, METHOD_RPC_NEGOTIATE, METHOD_RUNS_LIST, METHOD_SETTINGS_GET,
    METHOD_SETTINGS_UPDATE, METHOD_SNAPSHOTS_LIST, METHOD_STATUS_GET, ProjectRegistryEntry,
    ProjectResult, ProjectsAddParams, ProjectsListResult, RPC_JSONRPC_VERSION,
    RPC_PROTOCOL_VERSION, RecoverOpenParams, RecoverOpenResult, ResourceProfile, RpcId, RpcRequest,
    RpcResponse, RpcVersionNegotiationParams, RpcVersionNegotiationResult, RunsListParams,
    RunsListResult, SettingsGetResult, SettingsUpdateParams, SettingsUpdateResult,
    SnapshotsListParams, SnapshotsListResult, StatusGetParams, StatusGetResult, StoredSnapshot,
    UnixIpcConnection, VerificationDetails, WorkspaceState, detect_platform_identity,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{
    Emitter, Manager, Runtime,
    menu::{Menu, MenuItem, PredefinedMenuItem, Submenu},
    tray::TrayIconBuilder,
};

static EVENT_BRIDGE_RPC_ID: AtomicU64 = AtomicU64::new(1);
const TRAY_OPEN_ID: &str = "open-devrelay";
const TRAY_REFRESH_ID: &str = "refresh-state";
const TRAY_PAUSE_BACKGROUND_ID: &str = "pause-background";
const TRAY_HANDOFF_PREFIX: &str = "handoff-target|";
const TRAY_RUN_PREFIX: &str = "run-target|";
const TRAY_TARGET_SEPARATOR: char = '|';

#[derive(Debug, Serialize)]
struct RuntimeStatus {
    platform_key: String,
    architecture: String,
    devrelay_home: String,
    agent_socket_path: String,
    agent_socket_exists: bool,
}

#[derive(Debug, Serialize)]
struct AgentUiStatus {
    connected: bool,
    socket_path: String,
    methods: Vec<String>,
    health: Option<Value>,
    errors: Vec<String>,
}

#[derive(Debug, Serialize)]
struct UiBootstrap {
    runtime: RuntimeStatus,
    agent: AgentUiStatus,
    projects: Vec<ProjectRegistryEntry>,
    snapshots: Vec<StoredSnapshot>,
    leases: Vec<LeaseRecord>,
    handoffs: Vec<HandoffStatus>,
    devices: Vec<UiDeviceIdentity>,
    runs: Vec<devrelay_core::TaskRunRecord>,
    activity: Vec<devrelay_core::AuditEventRecord>,
    settings: Option<SettingsGetResult>,
}

#[derive(Debug, Clone, Default, Serialize)]
struct DeviceResourceSummary {
    cpu: Option<String>,
    memory: Option<String>,
    disk: Option<String>,
    power: Option<String>,
    cache_warmth: Option<String>,
}

#[derive(Debug, Serialize)]
struct UiDeviceIdentity {
    #[serde(flatten)]
    identity: devrelay_core::DeviceIdentity,
    resource_summary: DeviceResourceSummary,
}

#[derive(Debug, Serialize)]
struct UiOperationResult<T: Serialize> {
    ok: bool,
    message: String,
    data: Option<T>,
}

#[derive(Debug, Clone, Serialize)]
struct TrayNotice {
    message: String,
    kind: String,
}

#[derive(Debug, Clone, Serialize)]
struct TrayRunTargetPayload {
    project_id: String,
    target_device_id: String,
    target_label: String,
}

#[derive(Debug, Serialize)]
struct HandoffContinueResult {
    snapshot_id: Option<String>,
    verification: Option<VerificationDetails>,
    handoff: HandoffMutationResult,
}

#[tauri::command]
fn runtime_status() -> RuntimeStatus {
    build_runtime_status()
}

#[tauri::command]
fn ui_bootstrap() -> UiBootstrap {
    build_ui_bootstrap()
}

#[tauri::command]
fn project_add(path: String, manifest: Option<String>) -> UiOperationResult<ProjectRegistryEntry> {
    let path = path.trim();
    if path.is_empty() {
        return operation_error("project path is required".to_string());
    }
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return operation_error("project path must be absolute".to_string());
    }
    let manifest = manifest
        .and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        })
        .map(PathBuf::from);
    if let Some(manifest) = &manifest
        && !manifest.is_absolute()
    {
        return operation_error("manifest path must be absolute".to_string());
    }
    let params = ProjectsAddParams { path, manifest };
    match call_agent::<_, ProjectResult>(
        &resolved_home().agent_socket_path(),
        METHOD_PROJECTS_ADD,
        params,
    ) {
        Ok(result) => UiOperationResult {
            ok: true,
            message: "project added".to_string(),
            data: Some(result.project),
        },
        Err(err) => operation_error(format!("project add failed: {err}")),
    }
}

#[tauri::command]
fn recover_open(
    project_id: String,
    snapshot_id: String,
    path: String,
    name: Option<String>,
    register: bool,
) -> UiOperationResult<RecoverOpenResult> {
    let path = path.trim();
    if path.is_empty() {
        return operation_error("recovery path is required".to_string());
    }
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return operation_error("recovery path must be absolute".to_string());
    }
    let snapshot_id = snapshot_id.trim();
    if snapshot_id.is_empty() {
        return operation_error("snapshot is required".to_string());
    }
    let name = name.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    let params = RecoverOpenParams {
        snapshot_id: snapshot_id.to_string(),
        path,
        project: Some(project_id),
        register,
        name,
    };
    match call_agent(
        &resolved_home().agent_socket_path(),
        METHOD_RECOVER_OPEN,
        params,
    ) {
        Ok(result) => UiOperationResult {
            ok: true,
            message: "recovery opened".to_string(),
            data: Some(result),
        },
        Err(err) => operation_error(format!("recovery open failed: {err}")),
    }
}

#[tauri::command]
fn checkpoint_create(project_id: String) -> UiOperationResult<CheckpointCreateResult> {
    let home = resolved_home();
    let socket = home.agent_socket_path();
    let projects: ProjectsListResult = match call_agent(&socket, METHOD_PROJECTS_LIST, json!({})) {
        Ok(result) => result,
        Err(err) => {
            return operation_error(format!("failed to list projects before checkpoint: {err}"));
        }
    };
    let Some(project) = projects
        .projects
        .into_iter()
        .find(|project| project.project_id == project_id)
    else {
        return operation_error(format!("unknown project {project_id}"));
    };
    let params = CheckpointCreateParams {
        repo: project.local_path,
        manifest: project.manifest_path,
        label: Some("desktop".to_string()),
        pin: false,
    };
    match call_agent(&socket, METHOD_CHECKPOINT_CREATE, params) {
        Ok(result) => UiOperationResult {
            ok: true,
            message: "checkpoint created".to_string(),
            data: Some(result),
        },
        Err(err) => operation_error(format!("checkpoint failed: {err}")),
    }
}

#[tauri::command]
fn handoff_prepare(
    project_id: String,
    target_device_id: String,
) -> UiOperationResult<HandoffMutationResult> {
    let socket = resolved_home().agent_socket_path();
    let settings: SettingsGetResult = match call_agent(&socket, METHOD_SETTINGS_GET, json!({})) {
        Ok(result) => result,
        Err(err) => return operation_error(format!("failed to load local device settings: {err}")),
    };
    if settings.device_id == target_device_id {
        return operation_error("target device must be different from this device".to_string());
    }

    let project = match project_by_id(&socket, &project_id) {
        Ok(project) => project,
        Err(err) => return operation_error(err),
    };
    let leases: LeasesListResult = match call_agent(
        &socket,
        METHOD_LEASES_LIST,
        LeasesListParams {
            project: Some(project_id.clone()),
        },
    ) {
        Ok(result) => result,
        Err(err) => return operation_error(format!("failed to load writer lease: {err}")),
    };
    let Some(lease) = leases.leases.into_iter().find(|lease| {
        lease.state == LeaseState::Active
            && lease.holder_device_id.as_deref() == Some(settings.device_id.as_str())
    }) else {
        return operation_error(
            "this device does not currently hold an active writer lease".to_string(),
        );
    };

    let checkpoint: CheckpointCreateResult = match call_agent(
        &socket,
        METHOD_CHECKPOINT_CREATE,
        CheckpointCreateParams {
            repo: project.local_path,
            manifest: project.manifest_path,
            label: Some("desktop-handoff".to_string()),
            pin: false,
        },
    ) {
        Ok(result) => result,
        Err(err) => return operation_error(format!("handoff checkpoint failed: {err}")),
    };
    let params = HandoffBeginParams {
        project: project_id,
        lease_id: lease.lease_id,
        target_device_id,
        source_generation: checkpoint.checkpoint.metadata.state_hash,
        ttl_seconds: None,
    };
    match call_agent(&socket, METHOD_HANDOFF_BEGIN, params) {
        Ok(result) => UiOperationResult {
            ok: true,
            message: "target preparation started".to_string(),
            data: Some(result),
        },
        Err(err) => operation_error(format!("handoff preparation failed: {err}")),
    }
}

#[tauri::command]
fn handoff_abort(
    project_id: String,
    handoff_id: String,
) -> UiOperationResult<HandoffMutationResult> {
    let socket = resolved_home().agent_socket_path();
    let params = HandoffIdParams {
        project: project_id,
        handoff_id,
    };
    match call_agent(&socket, METHOD_HANDOFF_ABORT, params) {
        Ok(result) => UiOperationResult {
            ok: true,
            message: "handoff aborted".to_string(),
            data: Some(result),
        },
        Err(err) => operation_error(format!("handoff abort failed: {err}")),
    }
}

#[tauri::command]
fn handoff_continue_here(
    project_id: String,
    handoff_id: String,
) -> UiOperationResult<HandoffContinueResult> {
    let socket = resolved_home().agent_socket_path();
    let settings: SettingsGetResult = match call_agent(&socket, METHOD_SETTINGS_GET, json!({})) {
        Ok(result) => result,
        Err(err) => return operation_error(format!("failed to load local device settings: {err}")),
    };
    let project = match project_by_id(&socket, &project_id) {
        Ok(project) => project,
        Err(err) => return operation_error(err),
    };
    let status = match find_handoff_status(&socket, &project_id, &handoff_id) {
        Ok(status) => status,
        Err(err) => return operation_error(err),
    };
    let mut handoff = status.record;
    let mut journal = status.journal;
    if handoff.target_device_id != settings.device_id {
        return operation_error("handoff is waiting for a different target device".to_string());
    }

    let mut applied_snapshot_id = None;
    let mut verification = None;
    if handoff.state == HandoffState::TargetPrepare {
        let snapshots: SnapshotsListResult = match call_agent(
            &socket,
            METHOD_SNAPSHOTS_LIST,
            SnapshotsListParams {
                project: project_id.clone(),
            },
        ) {
            Ok(result) => result,
            Err(err) => {
                return operation_error(format!("failed to load handoff checkpoint: {err}"));
            }
        };
        let Some(snapshot) =
            snapshot_matching_generation(&snapshots.snapshots, &handoff.source_generation)
        else {
            return operation_error(
                "handoff checkpoint is not available on this device".to_string(),
            );
        };
        applied_snapshot_id = Some(snapshot.snapshot_id.clone());
        let apply: ApplySnapshotResult = match call_agent(
            &socket,
            METHOD_APPLY_SNAPSHOT,
            ApplySnapshotParams {
                repo: project.local_path,
                project: project_id.clone(),
                snapshot_id: snapshot.snapshot_id.clone(),
                dry_run: false,
            },
        ) {
            Ok(result) => result,
            Err(err) => return operation_error(format!("target apply failed: {err}")),
        };
        verification = apply.verification;
        let result = match mutate_handoff(
            &socket,
            METHOD_HANDOFF_TARGET_VERIFY,
            &project_id,
            &handoff_id,
        ) {
            Ok(result) => result,
            Err(err) => return operation_error(format!("target verification failed: {err}")),
        };
        handoff = result.handoff;
        journal = result.journal;
    }

    if handoff.state == HandoffState::TargetVerified {
        let result = match mutate_handoff(
            &socket,
            METHOD_HANDOFF_SOURCE_READY,
            &project_id,
            &handoff_id,
        ) {
            Ok(result) => result,
            Err(err) => return operation_error(format!("source readiness failed: {err}")),
        };
        handoff = result.handoff;
        journal = result.journal;
    }

    let result = match handoff.state {
        HandoffState::SourceReady => match call_agent(
            &socket,
            METHOD_HANDOFF_COMMIT,
            HandoffCommitParams {
                project: project_id,
                handoff_id,
                observed_source_generation: handoff.source_generation.clone(),
            },
        ) {
            Ok(result) => result,
            Err(err) => return operation_error(format!("handoff commit failed: {err}")),
        },
        HandoffState::Committed => HandoffMutationResult { handoff, journal },
        HandoffState::Aborted => {
            return operation_error("handoff has already been aborted".to_string());
        }
        HandoffState::TargetPrepare | HandoffState::TargetVerified => {
            return operation_error(format!(
                "handoff stopped before commit in {} state",
                handoff.state.as_str()
            ));
        }
    };

    UiOperationResult {
        ok: true,
        message: "continuation verified".to_string(),
        data: Some(HandoffContinueResult {
            snapshot_id: applied_snapshot_id,
            verification,
            handoff: result,
        }),
    }
}

#[tauri::command]
fn project_status(project_id: String) -> UiOperationResult<StatusGetResult> {
    let home = resolved_home();
    let socket = home.agent_socket_path();
    let project = match project_by_id(&socket, &project_id) {
        Ok(project) => project,
        Err(err) => return operation_error(err),
    };
    let params = StatusGetParams {
        repo: project.local_path,
        manifest: project.manifest_path,
    };
    match call_agent(&socket, METHOD_STATUS_GET, params) {
        Ok(result) => UiOperationResult {
            ok: true,
            message: "project status loaded".to_string(),
            data: Some(result),
        },
        Err(err) => operation_error(format!("status failed: {err}")),
    }
}

#[tauri::command]
fn open_project(project_id: String) -> UiOperationResult<String> {
    let socket = resolved_home().agent_socket_path();
    let project = match project_by_id(&socket, &project_id) {
        Ok(project) => project,
        Err(err) => return operation_error(err),
    };
    let path = project.local_path;
    let display_path = path.display().to_string();
    let spawn_result = platform_open_path(&path);
    match spawn_result {
        Ok(()) => UiOperationResult {
            ok: true,
            message: "project opened".to_string(),
            data: Some(display_path),
        },
        Err(err) => operation_error(format!("open project failed: {err}")),
    }
}

#[tauri::command]
fn settings_update(params: SettingsUpdateParams) -> UiOperationResult<SettingsUpdateResult> {
    let socket = resolved_home().agent_socket_path();
    match call_agent(&socket, METHOD_SETTINGS_UPDATE, params) {
        Ok(result) => UiOperationResult {
            ok: true,
            message: "settings updated".to_string(),
            data: Some(result),
        },
        Err(err) => operation_error(format!("settings update failed: {err}")),
    }
}

#[tauri::command]
fn diagnostics_export() -> UiOperationResult<DiagnosticsExportResult> {
    let socket = resolved_home().agent_socket_path();
    let params = DiagnosticsExportParams {
        out: None,
        include_sensitive_paths: false,
    };
    match call_agent(&socket, METHOD_DIAGNOSTICS_EXPORT, params) {
        Ok(result) => UiOperationResult {
            ok: true,
            message: "diagnostics exported".to_string(),
            data: Some(result),
        },
        Err(err) => operation_error(format!("diagnostics export failed: {err}")),
    }
}

fn build_ui_bootstrap() -> UiBootstrap {
    let runtime = build_runtime_status();
    let socket = resolved_home().agent_socket_path();
    let mut errors = Vec::new();
    let mut methods = Vec::new();
    let mut health = None;
    let mut projects = Vec::new();
    let mut snapshots = Vec::new();
    let mut leases = Vec::new();
    let mut handoffs = Vec::new();
    let mut devices = Vec::new();
    let mut runs = Vec::new();
    let mut activity = Vec::new();
    let mut settings = None;

    if runtime.agent_socket_exists {
        match call_agent::<_, RpcVersionNegotiationResult>(
            &socket,
            METHOD_RPC_NEGOTIATE,
            RpcVersionNegotiationParams {
                client_protocol_version: RPC_PROTOCOL_VERSION,
            },
        ) {
            Ok(result) => methods = result.methods,
            Err(err) => errors.push(format!("rpc.negotiate: {err}")),
        }
        match call_agent::<_, Value>(&socket, METHOD_AGENT_HEALTH, json!({})) {
            Ok(result) => health = Some(result),
            Err(err) => errors.push(format!("agent.health: {err}")),
        }
        match call_agent::<_, ProjectsListResult>(&socket, METHOD_PROJECTS_LIST, json!({})) {
            Ok(result) => projects = result.projects,
            Err(err) => errors.push(format!("projects.list: {err}")),
        }
        for project in &projects {
            match call_agent::<_, SnapshotsListResult>(
                &socket,
                METHOD_SNAPSHOTS_LIST,
                SnapshotsListParams {
                    project: project.project_id.clone(),
                },
            ) {
                Ok(result) => snapshots.extend(result.snapshots),
                Err(err) => errors.push(format!("snapshots.list {}: {err}", project.project_id)),
            }
        }
        match call_agent::<_, LeasesListResult>(
            &socket,
            METHOD_LEASES_LIST,
            LeasesListParams { project: None },
        ) {
            Ok(result) => leases = result.leases,
            Err(err) => errors.push(format!("leases.list: {err}")),
        }
        match call_agent::<_, HandoffsListResult>(
            &socket,
            METHOD_HANDOFFS_LIST,
            HandoffsListParams {
                project: None,
                include_journal: true,
            },
        ) {
            Ok(result) => handoffs = result.handoffs,
            Err(err) => errors.push(format!("handoffs.list: {err}")),
        }
        match call_agent::<_, DevicesListResult>(&socket, METHOD_DEVICES_LIST, json!({})) {
            Ok(result) => devices = result.devices,
            Err(err) => errors.push(format!("devices.list: {err}")),
        }
        match call_agent::<_, RunsListResult>(
            &socket,
            METHOD_RUNS_LIST,
            RunsListParams {
                project: None,
                limit: Some(100),
            },
        ) {
            Ok(result) => runs = result.runs,
            Err(err) => errors.push(format!("runs.list: {err}")),
        }
        match call_agent::<_, ActivityListResult>(
            &socket,
            METHOD_ACTIVITY_LIST,
            ActivityListParams {
                project: None,
                limit: Some(100),
            },
        ) {
            Ok(result) => activity = result.events,
            Err(err) => errors.push(format!("activity.list: {err}")),
        }
        match call_agent::<_, SettingsGetResult>(&socket, METHOD_SETTINGS_GET, json!({})) {
            Ok(result) => settings = Some(result),
            Err(err) => errors.push(format!("settings.get: {err}")),
        }
    } else {
        errors.push("agent socket is not available".to_string());
    }

    let device_views =
        build_device_views(devices, settings.as_ref(), &runtime, !snapshots.is_empty());

    UiBootstrap {
        runtime,
        agent: AgentUiStatus {
            connected: socket.exists() && health.is_some(),
            socket_path: socket.display().to_string(),
            methods,
            health,
            errors,
        },
        projects,
        snapshots,
        leases,
        handoffs,
        devices: device_views,
        runs,
        activity,
        settings,
    }
}

fn build_device_views(
    devices: Vec<devrelay_core::DeviceIdentity>,
    settings: Option<&SettingsGetResult>,
    runtime: &RuntimeStatus,
    snapshot_metadata_ready: bool,
) -> Vec<UiDeviceIdentity> {
    let local_device_id = settings.map(|settings| settings.device_id.as_str());
    devices
        .into_iter()
        .map(|identity| {
            let resource_summary = if Some(identity.device_id.as_str()) == local_device_id {
                local_resource_summary(runtime, snapshot_metadata_ready)
            } else {
                DeviceResourceSummary::default()
            };
            UiDeviceIdentity {
                identity,
                resource_summary,
            }
        })
        .collect()
}

fn local_resource_summary(
    runtime: &RuntimeStatus,
    snapshot_metadata_ready: bool,
) -> DeviceResourceSummary {
    let context = devrelay_core::ResourcePolicyContext::detect_current();
    let power_source = match context.power_source {
        devrelay_core::ResourcePowerSource::Ac => "AC",
        devrelay_core::ResourcePowerSource::Battery => "Battery",
        devrelay_core::ResourcePowerSource::Unknown => "Power unknown",
    };
    let foreground_load = match context.foreground_load {
        devrelay_core::ForegroundLoad::Idle => "idle",
        devrelay_core::ForegroundLoad::Busy => "busy",
        devrelay_core::ForegroundLoad::Unknown => "load unknown",
    };
    DeviceResourceSummary {
        cpu: Some(format!("{} cores, {foreground_load}", context.parallelism)),
        memory: total_memory_summary(),
        disk: disk_summary(&runtime.devrelay_home),
        power: Some(format!(
            "{power_source}, low power {}",
            if context.low_power_mode { "on" } else { "off" }
        )),
        cache_warmth: Some(if snapshot_metadata_ready {
            "Checkpoint metadata ready".to_string()
        } else {
            "Identity metadata only".to_string()
        }),
    }
}

fn total_memory_summary() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let raw = String::from_utf8_lossy(&output.stdout);
        let bytes = raw.trim().parse::<u64>().ok()?;
        Some(format!("{} total", format_bytes(bytes)))
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

fn disk_summary(path: &str) -> Option<String> {
    let output = Command::new("df").args(["-k", path]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let row = raw.lines().nth(1)?;
    let columns: Vec<&str> = row.split_whitespace().collect();
    let total_kib = columns.get(1)?.parse::<u64>().ok()?;
    let available_kib = columns.get(3)?.parse::<u64>().ok()?;
    Some(format!(
        "{} free / {} total",
        format_bytes(available_kib.saturating_mul(1024)),
        format_bytes(total_kib.saturating_mul(1024))
    ))
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 10.0 || unit == 0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn project_by_id(
    socket: &std::path::Path,
    project_id: &str,
) -> Result<ProjectRegistryEntry, String> {
    let projects: ProjectsListResult = call_agent(socket, METHOD_PROJECTS_LIST, json!({}))?;
    projects
        .projects
        .into_iter()
        .find(|project| project.project_id == project_id)
        .ok_or_else(|| format!("unknown project {project_id}"))
}

fn find_handoff_status(
    socket: &std::path::Path,
    project_id: &str,
    handoff_id: &str,
) -> Result<HandoffStatus, String> {
    let handoffs: HandoffsListResult = call_agent(
        socket,
        METHOD_HANDOFFS_LIST,
        HandoffsListParams {
            project: Some(project_id.to_string()),
            include_journal: true,
        },
    )?;
    handoffs
        .handoffs
        .into_iter()
        .find(|status| status.record.handoff_id == handoff_id)
        .ok_or_else(|| format!("unknown handoff {handoff_id}"))
}

fn snapshot_matching_generation<'a>(
    snapshots: &'a [StoredSnapshot],
    source_generation: &str,
) -> Option<&'a StoredSnapshot> {
    snapshots
        .iter()
        .filter(|snapshot| snapshot.metadata.state_hash == source_generation)
        .max_by_key(|snapshot| snapshot.sequence_number)
}

fn mutate_handoff(
    socket: &std::path::Path,
    method: &str,
    project_id: &str,
    handoff_id: &str,
) -> Result<HandoffMutationResult, String> {
    call_agent(
        socket,
        method,
        HandoffIdParams {
            project: project_id.to_string(),
            handoff_id: handoff_id.to_string(),
        },
    )
}

fn platform_open_path(path: &std::path::Path) -> Result<(), std::io::Error> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn().map(|_| ())
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("explorer").arg(path).spawn().map(|_| ())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(path).spawn().map(|_| ())
    }
}

fn build_runtime_status() -> RuntimeStatus {
    let platform = detect_platform_identity();
    let home = resolved_home();
    let socket = home.agent_socket_path();

    RuntimeStatus {
        platform_key: platform.platform_key,
        architecture: platform.architecture,
        devrelay_home: home.root().display().to_string(),
        agent_socket_path: socket.display().to_string(),
        agent_socket_exists: socket.exists(),
    }
}

fn resolved_home() -> DevRelayHome {
    DevRelayHome::resolve().unwrap_or_else(|_| fallback_home())
}

fn fallback_home() -> DevRelayHome {
    let root = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Library")
        .join("Application Support")
        .join("DevRelay");
    DevRelayHome::new(root)
}

fn call_agent<P, R>(socket: &std::path::Path, method: &str, params: P) -> Result<R, String>
where
    P: serde::Serialize,
    R: DeserializeOwned,
{
    AgentRpcClient::new(socket)
        .call(method, params)
        .map_err(|err| err.to_string())
}

fn operation_error<T: Serialize>(message: String) -> UiOperationResult<T> {
    UiOperationResult {
        ok: false,
        message,
        data: None,
    }
}

fn build_tray_menu<R: Runtime, M: Manager<R>>(
    manager: &M,
    state: &UiBootstrap,
) -> tauri::Result<Menu<R>> {
    let now = unix_now_seconds();
    let active_project = tray_active_project(state);
    let project_status = MenuItem::with_id(
        manager,
        "tray-status-project",
        tray_project_label(active_project),
        false,
        None::<&str>,
    )?;
    let checkpoint_status = MenuItem::with_id(
        manager,
        "tray-status-checkpoint",
        tray_checkpoint_label(state, active_project, now),
        false,
        None::<&str>,
    )?;
    let protection_status = MenuItem::with_id(
        manager,
        "tray-status-protection",
        tray_protection_label(state, active_project),
        false,
        None::<&str>,
    )?;
    let continue_submenu = build_tray_continue_submenu(manager, state, active_project, now)?;
    let run_submenu = build_tray_run_submenu(manager, state, active_project, now)?;
    let pause_label = if state
        .settings
        .as_ref()
        .is_some_and(|settings| settings.resource_profile == ResourceProfile::Eco)
    {
        "Resume background work"
    } else {
        "Pause background work"
    };
    let pause_background = MenuItem::with_id(
        manager,
        TRAY_PAUSE_BACKGROUND_ID,
        pause_label,
        state.settings.is_some() && state.agent.connected,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(manager, TRAY_OPEN_ID, "Open Dashboard", true, None::<&str>)?;
    let refresh = MenuItem::with_id(
        manager,
        TRAY_REFRESH_ID,
        "Refresh State",
        true,
        None::<&str>,
    )?;
    let separator_one = PredefinedMenuItem::separator(manager)?;
    let separator_two = PredefinedMenuItem::separator(manager)?;
    let separator_three = PredefinedMenuItem::separator(manager)?;
    let quit = PredefinedMenuItem::quit(manager, Some("Quit DevRelay"))?;
    let menu = Menu::new(manager)?;
    menu.append(&project_status)?;
    menu.append(&checkpoint_status)?;
    menu.append(&protection_status)?;
    menu.append(&separator_one)?;
    menu.append(&continue_submenu)?;
    menu.append(&run_submenu)?;
    menu.append(&pause_background)?;
    menu.append(&separator_two)?;
    menu.append(&open)?;
    menu.append(&refresh)?;
    menu.append(&separator_three)?;
    menu.append(&quit)?;
    Ok(menu)
}

fn build_tray_continue_submenu<R: Runtime, M: Manager<R>>(
    manager: &M,
    state: &UiBootstrap,
    active_project: Option<&ProjectRegistryEntry>,
    now: u64,
) -> tauri::Result<Submenu<R>> {
    let submenu = Submenu::with_id(manager, "tray-continue-submenu", "Continue on", true)?;
    let targets = tray_target_devices(state);
    if targets.is_empty() {
        let empty = MenuItem::with_id(
            manager,
            "tray-continue-empty",
            "No paired targets",
            false,
            None::<&str>,
        )?;
        submenu.append(&empty)?;
        return Ok(submenu);
    }

    let blocker = tray_handoff_blocker(state, active_project);
    if let Some(reason) = blocker {
        let item = MenuItem::with_id(
            manager,
            "tray-continue-blocker",
            reason,
            false,
            None::<&str>,
        )?;
        submenu.append(&item)?;
    }

    for target in targets {
        let project_id = active_project
            .map(|project| project.project_id.as_str())
            .unwrap_or_default();
        let target_id = target.identity.device_id.as_str();
        let item = MenuItem::with_id(
            manager,
            tray_target_menu_id(TRAY_HANDOFF_PREFIX, project_id, target_id),
            tray_device_label(target, now),
            blocker.is_none(),
            None::<&str>,
        )?;
        submenu.append(&item)?;
    }
    Ok(submenu)
}

fn build_tray_run_submenu<R: Runtime, M: Manager<R>>(
    manager: &M,
    state: &UiBootstrap,
    active_project: Option<&ProjectRegistryEntry>,
    now: u64,
) -> tauri::Result<Submenu<R>> {
    let submenu = Submenu::with_id(manager, "tray-run-submenu", "Run elsewhere", true)?;
    let targets = tray_target_devices(state);
    if targets.is_empty() {
        let empty = MenuItem::with_id(
            manager,
            "tray-run-empty",
            "No paired targets",
            false,
            None::<&str>,
        )?;
        submenu.append(&empty)?;
        return Ok(submenu);
    }
    if active_project.is_none() {
        let item = MenuItem::with_id(
            manager,
            "tray-run-blocker",
            "Add a project first",
            false,
            None::<&str>,
        )?;
        submenu.append(&item)?;
    }

    for target in targets {
        let project_id = active_project
            .map(|project| project.project_id.as_str())
            .unwrap_or_default();
        let target_id = target.identity.device_id.as_str();
        let item = MenuItem::with_id(
            manager,
            tray_target_menu_id(TRAY_RUN_PREFIX, project_id, target_id),
            tray_device_label(target, now),
            active_project.is_some(),
            None::<&str>,
        )?;
        submenu.append(&item)?;
    }
    Ok(submenu)
}

fn tray_active_project<'a>(state: &'a UiBootstrap) -> Option<&'a ProjectRegistryEntry> {
    if let Some(local_device_id) = state
        .settings
        .as_ref()
        .map(|settings| settings.device_id.as_str())
    {
        if let Some(project) = state.projects.iter().find(|project| {
            project.workspaces.values().any(|workspace| {
                workspace.device_id == local_device_id && workspace.state == WorkspaceState::Active
            })
        }) {
            return Some(project);
        }
        if let Some(project) = state
            .projects
            .iter()
            .find(|project| has_local_active_lease(state, &project.project_id))
        {
            return Some(project);
        }
    }
    state.projects.first()
}

fn tray_project_label(project: Option<&ProjectRegistryEntry>) -> String {
    match project {
        Some(project) => format!("Project: {}", truncate_menu_text(&project.display_name, 44)),
        None => "Project: none".to_string(),
    }
}

fn tray_checkpoint_label(
    state: &UiBootstrap,
    project: Option<&ProjectRegistryEntry>,
    now: u64,
) -> String {
    match project.and_then(|project| latest_snapshot_for_project(state, &project.project_id)) {
        Some(snapshot) => format!(
            "Checkpoint: {}",
            format_tray_age(snapshot.created_at_unix_seconds, now)
        ),
        None => "Checkpoint: none".to_string(),
    }
}

fn tray_protection_label(state: &UiBootstrap, project: Option<&ProjectRegistryEntry>) -> String {
    let Some(project) = project else {
        return "Protection: No project".to_string();
    };
    let label = if !state.agent.connected {
        "Agent offline"
    } else if open_handoff_for_project(state, &project.project_id) {
        "Handoff in progress"
    } else if latest_snapshot_for_project(state, &project.project_id).is_some() {
        "Checkpoint available"
    } else if has_local_active_lease(state, &project.project_id) {
        "Needs checkpoint"
    } else {
        "Open project here"
    };
    format!("Protection: {label}")
}

fn latest_snapshot_for_project<'a>(
    state: &'a UiBootstrap,
    project_id: &str,
) -> Option<&'a StoredSnapshot> {
    state
        .snapshots
        .iter()
        .filter(|snapshot| snapshot.project_id == project_id)
        .max_by(|left, right| {
            left.created_at_unix_seconds
                .cmp(&right.created_at_unix_seconds)
                .then(left.sequence_number.cmp(&right.sequence_number))
        })
}

fn tray_target_devices(state: &UiBootstrap) -> Vec<&UiDeviceIdentity> {
    let local_device_id = state
        .settings
        .as_ref()
        .map(|settings| settings.device_id.as_str());
    let mut targets: Vec<&UiDeviceIdentity> = state
        .devices
        .iter()
        .filter(|device| Some(device.identity.device_id.as_str()) != local_device_id)
        .collect();
    targets.sort_by(|left, right| {
        left.identity
            .display_name
            .cmp(&right.identity.display_name)
            .then(left.identity.device_id.cmp(&right.identity.device_id))
    });
    targets
}

fn tray_device_label(device: &UiDeviceIdentity, now: u64) -> String {
    let name = truncate_menu_text(&device.identity.display_name, 36);
    if device.identity.last_seen_unix_seconds == 0 {
        return name;
    }
    format!(
        "{name} ({})",
        format_tray_age(device.identity.last_seen_unix_seconds, now)
    )
}

fn tray_handoff_blocker(
    state: &UiBootstrap,
    project: Option<&ProjectRegistryEntry>,
) -> Option<&'static str> {
    let project = match project {
        Some(project) => project,
        None => return Some("Add a project first"),
    };
    if !state.agent.connected {
        return Some("Start the local agent first");
    }
    if !tray_method_available(state, METHOD_HANDOFF_BEGIN)
        || !tray_method_available(state, METHOD_CHECKPOINT_CREATE)
    {
        return Some("Update the local agent first");
    }
    if open_handoff_for_project(state, &project.project_id) {
        return Some("Finish the current handoff first");
    }
    if !has_local_active_lease(state, &project.project_id) {
        return Some("Open this project here first");
    }
    None
}

fn tray_method_available(state: &UiBootstrap, method: &str) -> bool {
    state
        .agent
        .methods
        .iter()
        .any(|candidate| candidate == method)
}

fn has_local_active_lease(state: &UiBootstrap, project_id: &str) -> bool {
    let Some(local_device_id) = state
        .settings
        .as_ref()
        .map(|settings| settings.device_id.as_str())
    else {
        return false;
    };
    state.leases.iter().any(|lease| {
        lease.project_id == project_id
            && lease.state == LeaseState::Active
            && lease.holder_device_id.as_deref() == Some(local_device_id)
    })
}

fn open_handoff_for_project(state: &UiBootstrap, project_id: &str) -> bool {
    state.handoffs.iter().any(|handoff| {
        handoff.record.project_id == project_id && !handoff.record.state.is_terminal()
    })
}

fn tray_target_menu_id(prefix: &str, project_id: &str, target_device_id: &str) -> String {
    format!("{prefix}{project_id}{TRAY_TARGET_SEPARATOR}{target_device_id}")
}

fn parse_tray_target_id<'a>(id: &'a str, prefix: &str) -> Option<(&'a str, &'a str)> {
    let payload = id.strip_prefix(prefix)?;
    let (project_id, target_device_id) = payload.split_once(TRAY_TARGET_SEPARATOR)?;
    if project_id.is_empty() || target_device_id.is_empty() {
        return None;
    }
    Some((project_id, target_device_id))
}

fn truncate_menu_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let keep = max_chars.saturating_sub(3);
    format!("{}...", value.chars().take(keep).collect::<String>())
}

fn format_tray_age(created_at_unix_seconds: u64, now: u64) -> String {
    if created_at_unix_seconds == 0 {
        return "never".to_string();
    }
    let delta = now.saturating_sub(created_at_unix_seconds);
    if delta < 10 {
        "just now".to_string()
    } else if delta < 60 {
        format!("{delta}s ago")
    } else if delta < 3_600 {
        format!("{}m ago", delta / 60)
    } else if delta < 86_400 {
        format!("{}h ago", delta / 3_600)
    } else {
        format!("{}d ago", delta / 86_400)
    }
}

fn unix_now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn emit_tray_notice(app: &tauri::AppHandle, message: impl Into<String>, kind: &str) {
    let _ = app.emit(
        "devrelay-tray-notice",
        TrayNotice {
            message: message.into(),
            kind: kind.to_string(),
        },
    );
}

fn refresh_tray_menu(app: &tauri::AppHandle) {
    let state = build_ui_bootstrap();
    match build_tray_menu(app, &state) {
        Ok(menu) => {
            if let Some(tray) = app.tray_by_id("main")
                && let Err(error) = tray.set_menu(Some(menu))
            {
                emit_tray_notice(app, format!("Tray refresh failed: {error}"), "bad");
            }
        }
        Err(error) => emit_tray_notice(app, format!("Tray refresh failed: {error}"), "bad"),
    }
}

fn handle_tray_handoff(app: &tauri::AppHandle, project_id: &str, target_device_id: &str) {
    show_main_window(app);
    let result = handoff_prepare(project_id.to_string(), target_device_id.to_string());
    let kind = if result.ok { "good" } else { "bad" };
    emit_tray_notice(app, result.message, kind);
    refresh_tray_menu(app);
    let _ = app.emit("devrelay-tray-refresh", ());
}

fn handle_tray_run_elsewhere(app: &tauri::AppHandle, project_id: &str, target_device_id: &str) {
    let state = build_ui_bootstrap();
    let target_label = state
        .devices
        .iter()
        .find(|device| device.identity.device_id == target_device_id)
        .map(|device| device.identity.display_name.clone())
        .unwrap_or_else(|| target_device_id.to_string());
    let _ = app.emit(
        "devrelay-tray-open-runs",
        TrayRunTargetPayload {
            project_id: project_id.to_string(),
            target_device_id: target_device_id.to_string(),
            target_label,
        },
    );
    show_main_window(app);
}

fn handle_tray_background_toggle(app: &tauri::AppHandle) {
    let socket = resolved_home().agent_socket_path();
    let settings: SettingsGetResult = match call_agent(&socket, METHOD_SETTINGS_GET, json!({})) {
        Ok(settings) => settings,
        Err(error) => {
            emit_tray_notice(app, format!("Settings refresh failed: {error}"), "bad");
            return;
        }
    };
    let next_profile = if settings.resource_profile == ResourceProfile::Eco {
        ResourceProfile::Adaptive
    } else {
        ResourceProfile::Eco
    };
    let result = settings_update(SettingsUpdateParams {
        resource_profile: Some(next_profile),
        mdns_enabled: None,
        editor_command: None,
    });
    if result.ok {
        let message = if next_profile == ResourceProfile::Eco {
            "Background work moved to Eco profile"
        } else {
            "Background work resumed"
        };
        emit_tray_notice(app, message, "good");
    } else {
        emit_tray_notice(app, result.message, "bad");
    }
    refresh_tray_menu(app);
    let _ = app.emit("devrelay-tray-refresh", ());
}

fn spawn_tray_action<F>(name: &str, task: F)
where
    F: FnOnce() + Send + 'static,
{
    let _ = std::thread::Builder::new()
        .name(format!("devrelay-desktop-tray-{name}"))
        .spawn(task);
}

fn spawn_agent_event_bridge(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("devrelay-desktop-event-bridge".to_string())
        .spawn(move || {
            let mut cursor = EventReplayCursor::from_start();
            loop {
                match run_event_bridge_once(&app, &mut cursor) {
                    Ok(()) => {}
                    Err(error) => {
                        let _ = app.emit("devrelay-agent-disconnected", error);
                        std::thread::sleep(Duration::from_secs(2));
                    }
                }
            }
        })
        .expect("failed to spawn DevRelay event bridge");
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let state = build_ui_bootstrap();
    let menu = build_tray_menu(app, &state)?;

    let mut tray = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .tooltip("DevRelay")
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let id = event.id().as_ref();
            if id == TRAY_OPEN_ID {
                show_main_window(app);
            } else if id == TRAY_REFRESH_ID {
                show_main_window(app);
                let app = app.clone();
                spawn_tray_action("refresh", move || {
                    refresh_tray_menu(&app);
                    let _ = app.emit("devrelay-tray-refresh", ());
                });
            } else if id == TRAY_PAUSE_BACKGROUND_ID {
                let app = app.clone();
                spawn_tray_action("background-toggle", move || {
                    handle_tray_background_toggle(&app);
                });
            } else if let Some((project_id, target_device_id)) =
                parse_tray_target_id(id, TRAY_HANDOFF_PREFIX)
            {
                let app = app.clone();
                let project_id = project_id.to_string();
                let target_device_id = target_device_id.to_string();
                spawn_tray_action("handoff", move || {
                    handle_tray_handoff(&app, &project_id, &target_device_id);
                });
            } else if let Some((project_id, target_device_id)) =
                parse_tray_target_id(id, TRAY_RUN_PREFIX)
            {
                let app = app.clone();
                let project_id = project_id.to_string();
                let target_device_id = target_device_id.to_string();
                spawn_tray_action("run-shortcut", move || {
                    handle_tray_run_elsewhere(&app, &project_id, &target_device_id);
                });
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn run_event_bridge_once(
    app: &tauri::AppHandle,
    cursor: &mut EventReplayCursor,
) -> Result<(), String> {
    let socket = resolved_home().agent_socket_path();
    let limits = IpcLimits::default();
    let mut connection =
        UnixIpcConnection::connect(&socket, limits).map_err(|err| err.to_string())?;
    let id = RpcId::String(format!(
        "desktop-events-{}",
        EVENT_BRIDGE_RPC_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let request = RpcRequest {
        jsonrpc: RPC_JSONRPC_VERSION.to_string(),
        id: Some(id.clone()),
        method: METHOD_EVENTS_SUBSCRIBE.to_string(),
        params: serde_json::to_value(EventsSubscribeParams { cursor: *cursor })
            .map_err(|err| err.to_string())?,
    };
    let bytes = serde_json::to_vec(&request).map_err(|err| err.to_string())?;
    connection
        .write_message(&bytes, limits)
        .map_err(|err| err.to_string())?;

    let response_bytes = connection
        .read_message(limits)
        .map_err(|err| err.to_string())?;
    let response: RpcResponse =
        serde_json::from_slice(&response_bytes).map_err(|err| err.to_string())?;
    if response.id.as_ref() != Some(&id) {
        return Err("event subscription response ID mismatch".to_string());
    }
    if let Some(error) = response.error {
        return Err(format!("event subscription failed: {}", error.message));
    }
    let result: EventsSubscribeResult = serde_json::from_value(
        response
            .result
            .ok_or_else(|| "event subscription response missing result".to_string())?,
    )
    .map_err(|err| err.to_string())?;
    *cursor = result.cursor;
    let _ = app.emit("devrelay-agent-connected", &result);

    loop {
        let message_bytes = connection
            .read_message(limits)
            .map_err(|err| err.to_string())?;
        let message: EventStreamMessage =
            serde_json::from_slice(&message_bytes).map_err(|err| err.to_string())?;
        match message {
            EventStreamMessage::Event { event } => {
                cursor.after_sequence = Some(event.sequence);
                let _ = app.emit("devrelay-agent-event", &event);
            }
            EventStreamMessage::Gap { gap } => {
                let _ = app.emit("devrelay-agent-gap", &gap);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_target_ids_round_trip() {
        let id = tray_target_menu_id(TRAY_HANDOFF_PREFIX, "project-1", "device-2");

        assert_eq!(
            parse_tray_target_id(&id, TRAY_HANDOFF_PREFIX),
            Some(("project-1", "device-2"))
        );
        assert_eq!(parse_tray_target_id(&id, TRAY_RUN_PREFIX), None);
    }

    #[test]
    fn tray_target_id_rejects_empty_parts() {
        assert_eq!(
            parse_tray_target_id("handoff-target|project-1|", TRAY_HANDOFF_PREFIX),
            None
        );
        assert_eq!(
            parse_tray_target_id("handoff-target||device-2", TRAY_HANDOFF_PREFIX),
            None
        );
    }

    #[test]
    fn tray_age_uses_compact_labels() {
        let now = 200_000;
        assert_eq!(format_tray_age(0, now), "never");
        assert_eq!(format_tray_age(now - 5, now), "just now");
        assert_eq!(format_tray_age(now - 50, now), "50s ago");
        assert_eq!(format_tray_age(now - 120, now), "2m ago");
        assert_eq!(format_tray_age(now - 7_200, now), "2h ago");
        assert_eq!(format_tray_age(now - 172_800, now), "2d ago");
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            setup_tray(app)?;
            spawn_agent_event_bridge(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            runtime_status,
            ui_bootstrap,
            project_add,
            recover_open,
            checkpoint_create,
            handoff_prepare,
            handoff_continue_here,
            handoff_abort,
            project_status,
            open_project,
            settings_update,
            diagnostics_export,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run DevRelay desktop app");
}
