use devrelay_core::{
    ActivityListParams, ActivityListResult, AgentRpcClient, CheckpointCreateParams,
    CheckpointCreateResult, DevRelayHome, DevicesListResult, DiagnosticsExportParams,
    DiagnosticsExportResult, EventReplayCursor, EventStreamMessage, EventsSubscribeParams,
    EventsSubscribeResult, IpcConnection, IpcLimits, METHOD_ACTIVITY_LIST, METHOD_AGENT_HEALTH,
    METHOD_CHECKPOINT_CREATE, METHOD_DEVICES_LIST, METHOD_DIAGNOSTICS_EXPORT,
    METHOD_EVENTS_SUBSCRIBE, METHOD_PROJECTS_LIST, METHOD_RPC_NEGOTIATE, METHOD_RUNS_LIST,
    METHOD_SETTINGS_GET, METHOD_SETTINGS_UPDATE, METHOD_STATUS_GET, ProjectRegistryEntry,
    ProjectsListResult, RPC_JSONRPC_VERSION, RPC_PROTOCOL_VERSION, RpcId, RpcRequest, RpcResponse,
    RpcVersionNegotiationParams, RpcVersionNegotiationResult, RunsListParams, RunsListResult,
    SettingsGetResult, SettingsUpdateParams, SettingsUpdateResult, StatusGetParams,
    StatusGetResult, UnixIpcConnection, detect_platform_identity,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tauri::Emitter;

static EVENT_BRIDGE_RPC_ID: AtomicU64 = AtomicU64::new(1);

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
    devices: Vec<devrelay_core::DeviceIdentity>,
    runs: Vec<devrelay_core::TaskRunRecord>,
    activity: Vec<devrelay_core::AuditEventRecord>,
    settings: Option<SettingsGetResult>,
}

#[derive(Debug, Serialize)]
struct UiOperationResult<T: Serialize> {
    ok: bool,
    message: String,
    data: Option<T>,
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
        devices,
        runs,
        activity,
        settings,
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

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            spawn_agent_event_bridge(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            runtime_status,
            ui_bootstrap,
            checkpoint_create,
            project_status,
            open_project,
            settings_update,
            diagnostics_export,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run DevRelay desktop app");
}
