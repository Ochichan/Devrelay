use anyhow::Context;
use clap::{Parser, ValueEnum};
#[cfg(unix)]
use devrelay_core::{
    ActivityListParams, ActivityListResult, AnchorSnapshotRepo, ApplySnapshotParams,
    ApplySnapshotResult, AuditEventInput, AuditEventType, AuditOutcome, CheckpointCreateParams,
    CheckpointCreateResult, DevicesListResult, DiagnosticsExportParams, DiagnosticsExportResult,
    EventEnvelope, EventReplayCursor, EventSequence, EventStreamMessage, EventsSubscribeParams,
    EventsSubscribeResult, GitRepo, HandoffBeginParams, HandoffCommitParams, HandoffIdParams,
    HandoffMutationResult, HandoffRecord, HandoffRecoverParams, HandoffRecoverResult,
    HandoffStatus, HandoffsListParams, HandoffsListResult, IpcConnection, IpcLimits, IpcTransport,
    Manifest, ProjectRegistryEntry, ProjectResult, ProjectsAddParams, ProjectsListResult,
    ProjectsRemoveParams, ProjectsShowParams, RPC_PROTOCOL_VERSION, RecoverListParams,
    RecoverListResult, RecoverOpenParams, RecoverOpenResult, RecoverShowParams, RecoverShowResult,
    RpcError, RpcId, RpcRequest, RpcResponse, RpcVersionNegotiationParams,
    RpcVersionNegotiationResult, RunsListParams, RunsListResult, SettingsGetResult,
    SettingsUpdateParams, SettingsUpdateResult, SnapshotApplyStartedEvent,
    SnapshotApplyVerifiedEvent, SnapshotLocalCreatedEvent, SnapshotStore, SnapshotsListParams,
    SnapshotsListResult, StatusGetParams, StatusGetResult, StoredSnapshot, StructuredLogFile,
    StructuredLogRecord, TypedEventPayload, UnixIpcConnection, UnixIpcListener,
    WorkspaceRegistryEntry, WorkspaceState, WorkspaceStateChangedEvent, apply_snapshot,
    classify_untracked_paths, plan_apply_snapshot, workspace_id_for,
};
use devrelay_core::{
    AgentRole, AnchorLayout, AnchorMode, DevRelayHome, LocalConfig, LogRedactor,
    METHOD_ACTIVITY_LIST, METHOD_AGENT_HEALTH, METHOD_APPLY_SNAPSHOT, METHOD_CHECKPOINT_CREATE,
    METHOD_DEVICES_LIST, METHOD_DIAGNOSTICS_EXPORT, METHOD_EVENTS_SUBSCRIBE, METHOD_HANDOFF_ABORT,
    METHOD_HANDOFF_BEGIN, METHOD_HANDOFF_COMMIT, METHOD_HANDOFF_RECOVER,
    METHOD_HANDOFF_SOURCE_READY, METHOD_HANDOFF_TARGET_VERIFY, METHOD_HANDOFFS_LIST,
    METHOD_PROJECTS_ADD, METHOD_PROJECTS_LIST, METHOD_PROJECTS_REMOVE, METHOD_PROJECTS_SHOW,
    METHOD_RECOVER_LIST, METHOD_RECOVER_OPEN, METHOD_RECOVER_SHOW, METHOD_RPC_NEGOTIATE,
    METHOD_RUNS_LIST, METHOD_SETTINGS_GET, METHOD_SETTINGS_UPDATE, METHOD_SNAPSHOTS_LIST,
    METHOD_STATUS_GET, MetadataDb, StructuredLogLevel, current_platform_key,
};
use serde::Serialize;
#[cfg(unix)]
use std::collections::BTreeMap;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc,
};
use std::thread;
use std::time::Duration;
#[cfg(unix)]
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Parser)]
#[command(name = "devrelay-agent")]
#[command(about = "DevRelay local agent")]
#[command(version)]
struct Cli {
    #[arg(long)]
    foreground: bool,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    socket_path: Option<PathBuf>,
    #[arg(long, value_enum, default_value = "info")]
    log_level: LogLevel,
    #[arg(long)]
    health: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

#[cfg(unix)]
const DEFAULT_HANDOFF_TTL_SECONDS: u64 = 10 * 60;
#[cfg(unix)]
const MAX_HANDOFF_TTL_SECONDS: u64 = 24 * 60 * 60;

impl LogLevel {
    fn structured(self) -> StructuredLogLevel {
        match self {
            Self::Error => StructuredLogLevel::Error,
            Self::Warn => StructuredLogLevel::Warn,
            Self::Info => StructuredLogLevel::Info,
            Self::Debug => StructuredLogLevel::Debug,
            Self::Trace => StructuredLogLevel::Trace,
        }
    }

    fn enabled(self, level: StructuredLogLevel) -> bool {
        level <= self.structured()
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

#[derive(Debug, Serialize)]
struct AgentHealth {
    status: &'static str,
    role: AgentRole,
    anchor_mode: AnchorMode,
    foreground: bool,
    config_path: PathBuf,
    socket_path: PathBuf,
    anchor: Option<AnchorLayout>,
    project_count: usize,
    database_path: PathBuf,
    shutdown_requested: bool,
}

#[derive(Clone)]
struct AgentState {
    foreground: bool,
    home: DevRelayHome,
    role: AgentRole,
    anchor_layout: Option<AnchorLayout>,
    config_path: PathBuf,
    socket_path: PathBuf,
    config: Arc<Mutex<LocalConfig>>,
    database_path: PathBuf,
    shutdown: Arc<AtomicBool>,
    #[cfg(unix)]
    events: Arc<AgentEventLog>,
    #[cfg(unix)]
    logger: AgentLogger,
    #[cfg(unix)]
    next_operation_id: Arc<AtomicU64>,
}

#[cfg(unix)]
#[derive(Clone)]
struct AgentLogger {
    level: LogLevel,
    file: Arc<Mutex<StructuredLogFile>>,
}

#[cfg(unix)]
impl AgentLogger {
    fn new(level: LogLevel, path: PathBuf) -> Self {
        Self {
            level,
            file: Arc::new(Mutex::new(StructuredLogFile::new(path))),
        }
    }

    fn log(&self, record: StructuredLogRecord) {
        if !self.level.enabled(record.level) {
            return;
        }
        if let Ok(file) = self.file.lock()
            && let Err(err) = file.append(&record)
        {
            eprintln!("devrelay-agent log write error: {err}");
        }
        eprintln!(
            "{}",
            record.to_human_line(&devrelay_core::LogRedactor::new())
        );
    }

    fn log_message(&self, level: StructuredLogLevel, target: &'static str, message: &'static str) {
        self.log(StructuredLogRecord::new(level, target, message));
    }
}

impl AgentState {
    fn health(&self) -> AgentHealth {
        AgentHealth {
            status: "ok",
            role: self.role,
            anchor_mode: self.anchor_mode(),
            foreground: self.foreground,
            config_path: self.config_path.clone(),
            socket_path: self.socket_path.clone(),
            anchor: self.anchor_layout.clone(),
            project_count: self.project_count(),
            database_path: self.database_path.clone(),
            shutdown_requested: self.shutdown.load(Ordering::SeqCst),
        }
    }

    fn project_count(&self) -> usize {
        self.config
            .lock()
            .map(|config| config.project_registry.projects.len())
            .unwrap_or_default()
    }

    fn anchor_mode(&self) -> AnchorMode {
        self.config
            .lock()
            .map(|config| config.anchor_mode)
            .unwrap_or(AnchorMode::LocalOnly)
    }

    fn supported_methods() -> Vec<String> {
        vec![
            METHOD_RPC_NEGOTIATE.to_string(),
            METHOD_AGENT_HEALTH.to_string(),
            METHOD_STATUS_GET.to_string(),
            METHOD_PROJECTS_ADD.to_string(),
            METHOD_PROJECTS_LIST.to_string(),
            METHOD_PROJECTS_SHOW.to_string(),
            METHOD_PROJECTS_REMOVE.to_string(),
            METHOD_CHECKPOINT_CREATE.to_string(),
            METHOD_SNAPSHOTS_LIST.to_string(),
            METHOD_APPLY_SNAPSHOT.to_string(),
            METHOD_HANDOFFS_LIST.to_string(),
            METHOD_HANDOFF_BEGIN.to_string(),
            METHOD_HANDOFF_TARGET_VERIFY.to_string(),
            METHOD_HANDOFF_SOURCE_READY.to_string(),
            METHOD_HANDOFF_COMMIT.to_string(),
            METHOD_HANDOFF_ABORT.to_string(),
            METHOD_HANDOFF_RECOVER.to_string(),
            METHOD_RECOVER_LIST.to_string(),
            METHOD_RECOVER_SHOW.to_string(),
            METHOD_RECOVER_OPEN.to_string(),
            METHOD_DIAGNOSTICS_EXPORT.to_string(),
            METHOD_EVENTS_SUBSCRIBE.to_string(),
            METHOD_DEVICES_LIST.to_string(),
            METHOD_ACTIVITY_LIST.to_string(),
            METHOD_RUNS_LIST.to_string(),
            METHOD_SETTINGS_GET.to_string(),
            METHOD_SETTINGS_UPDATE.to_string(),
        ]
    }
}

#[cfg(unix)]
#[derive(Default)]
struct AgentEventLog {
    inner: Mutex<AgentEventLogInner>,
}

#[cfg(unix)]
struct AgentEventLogInner {
    next_sequence: u64,
    events: Vec<EventEnvelope>,
    subscribers: Vec<mpsc::Sender<EventEnvelope>>,
}

#[cfg(unix)]
impl Default for AgentEventLogInner {
    fn default() -> Self {
        Self {
            next_sequence: 1,
            events: Vec::new(),
            subscribers: Vec::new(),
        }
    }
}

#[cfg(unix)]
struct AgentEventSubscription {
    replay: Vec<EventEnvelope>,
    current_sequence: Option<EventSequence>,
    receiver: mpsc::Receiver<EventEnvelope>,
}

#[cfg(unix)]
impl AgentEventLog {
    fn publish<T: TypedEventPayload>(&self, payload: T) -> anyhow::Result<EventEnvelope> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|err| anyhow::anyhow!("event log lock poisoned: {err}"))?;
        let sequence = EventSequence::new(inner.next_sequence)
            .ok_or_else(|| anyhow::anyhow!("event sequence exhausted"))?;
        inner.next_sequence = inner
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("event sequence exhausted"))?;

        let event = EventEnvelope::with_typed_payload(sequence, payload)?;
        inner.events.push(event.clone());
        inner
            .subscribers
            .retain(|sender| sender.send(event.clone()).is_ok());
        Ok(event)
    }

    fn subscribe(&self, cursor: EventReplayCursor) -> anyhow::Result<AgentEventSubscription> {
        let (sender, receiver) = mpsc::channel();
        let mut inner = self
            .inner
            .lock()
            .map_err(|err| anyhow::anyhow!("event log lock poisoned: {err}"))?;
        let replay = inner
            .events
            .iter()
            .filter(|event| cursor.accepts(event.sequence))
            .cloned()
            .collect::<Vec<_>>();
        let current_sequence = inner.current_sequence();
        inner.subscribers.push(sender);
        Ok(AgentEventSubscription {
            replay,
            current_sequence,
            receiver,
        })
    }
}

#[cfg(unix)]
impl AgentEventLogInner {
    fn current_sequence(&self) -> Option<EventSequence> {
        if self.next_sequence == 1 {
            None
        } else {
            EventSequence::new(self.next_sequence - 1)
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let shutdown = install_shutdown_handler()?;
    let home = DevRelayHome::resolve()?;
    home.create_base_dirs()?;
    let config_path = cli.config.clone().unwrap_or_else(|| home.config_file());
    let mut config = load_or_create_config(&config_path)?;
    config.mark_device_seen_now();
    config
        .save(&config_path)
        .with_context(|| format!("failed to save {}", config_path.display()))?;
    let role = AgentRole::from_anchor_mode(config.anchor_mode);
    if role == AgentRole::Anchor {
        home.create_anchor_dirs()?;
    }
    let database_path = metadata_database_path_for_role(&home, role);
    let db = MetadataDb::open(&database_path)?;
    db.upsert_device_identity(&config.device_identity())?;
    let anchor_layout = (role == AgentRole::Anchor).then(|| home.anchor_layout());
    let socket_path = cli
        .socket_path
        .clone()
        .unwrap_or_else(|| home.agent_socket_path());

    #[cfg(unix)]
    let logger = AgentLogger::new(cli.log_level, home.log_dir().join("agent.log"));
    #[cfg(unix)]
    logger.log(
        StructuredLogRecord::new(
            StructuredLogLevel::Info,
            "agent.lifecycle",
            "devrelay-agent started",
        )
        .with_field("foreground", cli.foreground.to_string())
        .with_field("log_level", cli.log_level.as_str())
        .with_field("role", role.label())
        .with_field(
            "project_count",
            config.project_registry.projects.len().to_string(),
        )
        .with_field("socket_path", socket_path.to_string_lossy().to_string()),
    );

    let state = AgentState {
        foreground: cli.foreground,
        home,
        role,
        anchor_layout,
        config_path,
        socket_path,
        config: Arc::new(Mutex::new(config)),
        database_path,
        shutdown: Arc::clone(&shutdown),
        #[cfg(unix)]
        events: Arc::new(AgentEventLog::default()),
        #[cfg(unix)]
        logger,
        #[cfg(unix)]
        next_operation_id: Arc::new(AtomicU64::new(1)),
    };

    if cli.health {
        println!("{}", serde_json::to_string_pretty(&state.health())?);
        return Ok(());
    }

    #[cfg(unix)]
    let _ipc_thread = if cli.foreground {
        Some(spawn_ipc_server(state.clone())?)
    } else {
        None
    };

    if cli.foreground {
        while !shutdown.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(100));
        }
        #[cfg(unix)]
        state.logger.log_message(
            StructuredLogLevel::Info,
            "agent.lifecycle",
            "devrelay-agent shutdown requested",
        );
    }
    Ok(())
}

fn metadata_database_path_for_role(home: &DevRelayHome, role: AgentRole) -> PathBuf {
    match role {
        AgentRole::LocalOnly => home.root().join("agent.sqlite"),
        AgentRole::Anchor => home.anchor_metadata_db_path(),
    }
}

#[cfg(unix)]
fn spawn_ipc_server(state: AgentState) -> anyhow::Result<thread::JoinHandle<()>> {
    let listener = UnixIpcListener::bind(&state.socket_path)
        .with_context(|| format!("failed to bind IPC socket {}", state.socket_path.display()))?;
    thread::Builder::new()
        .name("devrelay-agent-ipc".to_string())
        .spawn(move || run_ipc_server(listener, state))
        .context("failed to spawn IPC server thread")
}

#[cfg(unix)]
fn run_ipc_server(listener: UnixIpcListener, state: AgentState) {
    while !state.shutdown.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok(connection) => {
                let connection_state = state.clone();
                let _ = thread::Builder::new()
                    .name("devrelay-agent-rpc".to_string())
                    .spawn(move || handle_rpc_connection(connection, connection_state));
            }
            Err(err) => state.logger.log(
                StructuredLogRecord::new(StructuredLogLevel::Warn, "agent.ipc", "IPC accept error")
                    .with_field("error", err.to_string()),
            ),
        }
    }
}

#[cfg(unix)]
fn handle_rpc_connection(mut connection: UnixIpcConnection, state: AgentState) {
    let response = match connection.read_message(IpcLimits::default()) {
        Ok(bytes) => match RpcRequest::parse(&bytes) {
            Ok(request) if request.method == METHOD_EVENTS_SUBSCRIBE => {
                let operation_id = next_operation_id(&state);
                log_rpc_request(&state, &request, &operation_id);
                handle_events_subscribe_connection(connection, request, state, operation_id);
                return;
            }
            Ok(request) => {
                let operation_id = next_operation_id(&state);
                let request_id = request_id_string(request.id.as_ref());
                let method = request.method.clone();
                log_rpc_request(&state, &request, &operation_id);
                let response = handle_rpc_request(request, &state);
                log_rpc_response(
                    &state,
                    request_id.as_deref(),
                    &operation_id,
                    &method,
                    response.error.is_some(),
                );
                response
            }
            Err(error) => RpcResponse::error(None, error),
        },
        Err(err) => {
            state.logger.log(
                StructuredLogRecord::new(StructuredLogLevel::Warn, "agent.ipc", "IPC read error")
                    .with_field("error", err.to_string()),
            );
            return;
        }
    };

    write_ipc_json(&state.logger, &mut connection, &response);
}

#[cfg(unix)]
fn handle_events_subscribe_connection(
    mut connection: UnixIpcConnection,
    request: RpcRequest,
    state: AgentState,
    operation_id: String,
) {
    let id = match request.required_id() {
        Ok(id) => id,
        Err(error) => {
            let response = RpcResponse::error(None, error);
            write_ipc_json(&state.logger, &mut connection, &response);
            return;
        }
    };
    let request_id = request_id_string(Some(&id));
    let params = if request.params.is_null() {
        EventsSubscribeParams::default()
    } else {
        match serde_json::from_value::<EventsSubscribeParams>(request.params) {
            Ok(params) => params,
            Err(err) => {
                let response =
                    RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string()));
                write_ipc_json(&state.logger, &mut connection, &response);
                return;
            }
        }
    };
    let subscription = match state.events.subscribe(params.cursor) {
        Ok(subscription) => subscription,
        Err(err) => {
            let response = RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
            write_ipc_json(&state.logger, &mut connection, &response);
            return;
        }
    };
    let response = match serde_json::to_value(EventsSubscribeResult {
        cursor: params.cursor,
        replayed: subscription.replay.len(),
        current_sequence: subscription.current_sequence,
    }) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    log_rpc_response(
        &state,
        request_id.as_deref(),
        &operation_id,
        METHOD_EVENTS_SUBSCRIBE,
        response.error.is_some(),
    );
    if !write_ipc_json(&state.logger, &mut connection, &response) {
        return;
    }

    for event in subscription.replay {
        if !write_ipc_json(
            &state.logger,
            &mut connection,
            &EventStreamMessage::event(event),
        ) {
            return;
        }
    }

    while !state.shutdown.load(Ordering::SeqCst) {
        match subscription
            .receiver
            .recv_timeout(Duration::from_millis(100))
        {
            Ok(event) => {
                if !write_ipc_json(
                    &state.logger,
                    &mut connection,
                    &EventStreamMessage::event(event),
                ) {
                    return;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(unix)]
fn write_ipc_json<T: Serialize>(
    logger: &AgentLogger,
    connection: &mut UnixIpcConnection,
    value: &T,
) -> bool {
    let bytes = match serde_json::to_vec(value) {
        Ok(bytes) => bytes,
        Err(err) => {
            logger.log(
                StructuredLogRecord::new(
                    StructuredLogLevel::Error,
                    "agent.ipc",
                    "IPC JSON serialization error",
                )
                .with_field("error", err.to_string()),
            );
            return false;
        }
    };
    if let Err(err) = connection.write_message(&bytes, IpcLimits::default()) {
        logger.log(
            StructuredLogRecord::new(StructuredLogLevel::Warn, "agent.ipc", "IPC write error")
                .with_field("error", err.to_string()),
        );
        return false;
    }
    true
}

#[cfg(unix)]
fn next_operation_id(state: &AgentState) -> String {
    format!(
        "op-{}-{}",
        std::process::id(),
        state.next_operation_id.fetch_add(1, Ordering::SeqCst)
    )
}

#[cfg(unix)]
fn request_id_string(id: Option<&RpcId>) -> Option<String> {
    id.map(|id| match id {
        RpcId::String(value) => value.clone(),
        RpcId::Number(value) => value.to_string(),
    })
}

#[cfg(unix)]
fn log_rpc_request(state: &AgentState, request: &RpcRequest, operation_id: &str) {
    let mut record = StructuredLogRecord::new(
        StructuredLogLevel::Info,
        "agent.rpc",
        "RPC request received",
    )
    .with_operation_id(operation_id)
    .with_field("method", request.method.clone());
    if let Some(request_id) = request_id_string(request.id.as_ref()) {
        record = record.with_request_id(request_id);
    }
    state.logger.log(record);
}

#[cfg(unix)]
fn log_rpc_response(
    state: &AgentState,
    request_id: Option<&str>,
    operation_id: &str,
    method: &str,
    is_error: bool,
) {
    let mut record = StructuredLogRecord::new(
        if is_error {
            StructuredLogLevel::Warn
        } else {
            StructuredLogLevel::Info
        },
        "agent.rpc",
        "RPC response sent",
    )
    .with_operation_id(operation_id)
    .with_field("method", method.to_string())
    .with_field("status", if is_error { "error" } else { "ok" });
    if let Some(request_id) = request_id {
        record = record.with_request_id(request_id.to_string());
    }
    state.logger.log(record);
}

#[cfg(unix)]
fn handle_rpc_request(request: RpcRequest, state: &AgentState) -> RpcResponse {
    let id = match request.required_id() {
        Ok(id) => id,
        Err(error) => return RpcResponse::error(None, error),
    };

    match request.method.as_str() {
        METHOD_RPC_NEGOTIATE => handle_rpc_negotiate(id, request.params),
        METHOD_AGENT_HEALTH => match serde_json::to_value(state.health()) {
            Ok(result) => RpcResponse::success(id, result),
            Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
        },
        METHOD_STATUS_GET => handle_status_get(id, request.params),
        METHOD_PROJECTS_ADD => handle_projects_add(id, request.params, state),
        METHOD_PROJECTS_LIST => handle_projects_list(id, state),
        METHOD_PROJECTS_SHOW => handle_projects_show(id, request.params, state),
        METHOD_PROJECTS_REMOVE => handle_projects_remove(id, request.params, state),
        METHOD_CHECKPOINT_CREATE => handle_checkpoint_create(id, request.params, state),
        METHOD_SNAPSHOTS_LIST => handle_snapshots_list(id, request.params, state),
        METHOD_APPLY_SNAPSHOT => handle_apply_snapshot(id, request.params, state),
        METHOD_HANDOFFS_LIST => handle_handoffs_list(id, request.params, state),
        METHOD_HANDOFF_BEGIN => handle_handoff_begin(id, request.params, state),
        METHOD_HANDOFF_TARGET_VERIFY => handle_handoff_target_verify(id, request.params, state),
        METHOD_HANDOFF_SOURCE_READY => handle_handoff_source_ready(id, request.params, state),
        METHOD_HANDOFF_COMMIT => handle_handoff_commit(id, request.params, state),
        METHOD_HANDOFF_ABORT => handle_handoff_abort(id, request.params, state),
        METHOD_HANDOFF_RECOVER => handle_handoff_recover(id, request.params, state),
        METHOD_RECOVER_LIST => handle_recover_list(id, request.params, state),
        METHOD_RECOVER_SHOW => handle_recover_show(id, request.params, state),
        METHOD_RECOVER_OPEN => handle_recover_open(id, request.params, state),
        METHOD_DIAGNOSTICS_EXPORT => handle_diagnostics_export(id, request.params, state),
        METHOD_DEVICES_LIST => handle_devices_list(id, state),
        METHOD_ACTIVITY_LIST => handle_activity_list(id, request.params, state),
        METHOD_RUNS_LIST => handle_runs_list(id, request.params, state),
        METHOD_SETTINGS_GET => handle_settings_get(id, state),
        METHOD_SETTINGS_UPDATE => handle_settings_update(id, request.params, state),
        method => RpcResponse::error(Some(id), RpcError::method_not_found(method)),
    }
}

#[cfg(unix)]
fn handle_rpc_negotiate(id: devrelay_core::RpcId, params: serde_json::Value) -> RpcResponse {
    let params: RpcVersionNegotiationParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    if params.client_protocol_version != RPC_PROTOCOL_VERSION {
        return RpcResponse::error(
            Some(id),
            RpcError::version_mismatch(params.client_protocol_version),
        );
    }

    RpcResponse::success(
        id,
        serde_json::json!(RpcVersionNegotiationResult {
            protocol_version: RPC_PROTOCOL_VERSION,
            server_name: "devrelay-agent".to_string(),
            methods: AgentState::supported_methods(),
        }),
    )
}

#[cfg(unix)]
fn handle_status_get(id: devrelay_core::RpcId, params: serde_json::Value) -> RpcResponse {
    let params: StatusGetParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let repo = GitRepo::new(params.repo);
    let manifest_path = params
        .manifest
        .unwrap_or_else(|| repo.path().join("devrelay.toml"));
    let manifest = match Manifest::load(&manifest_path) {
        Ok(manifest) => manifest,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let status = match repo.status() {
        Ok(status) => status,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let untracked = match classify_untracked_paths(repo.path(), &manifest, status.untracked_paths())
    {
        Ok(untracked) => untracked,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let result = StatusGetResult {
        status: status.summary(),
        entries: status.entries,
        untracked,
    };

    match serde_json::to_value(result) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_checkpoint_create(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: CheckpointCreateParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let repo = GitRepo::new(params.repo);
    let manifest_path = params
        .manifest
        .unwrap_or_else(|| repo.path().join("devrelay.toml"));
    let manifest = match Manifest::load(&manifest_path) {
        Ok(manifest) => manifest,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let mut store = match SnapshotStore::open(&state.home, &manifest.project_id) {
        Ok(store) => store,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let checkpoint = match store.checkpoint(&repo, &manifest, params.pin, params.label) {
        Ok(checkpoint) => checkpoint,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    if let Err(err) = state
        .events
        .publish(SnapshotLocalCreatedEvent::from_snapshot(&checkpoint))
    {
        return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
    }
    let result = CheckpointCreateResult {
        checkpoint,
        snapshot_repo: store.snapshot_repo_path().to_path_buf(),
    };

    match serde_json::to_value(result) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_snapshots_list(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: SnapshotsListParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let store = match SnapshotStore::open(&state.home, &params.project) {
        Ok(store) => store,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let snapshots = match store.list_snapshots() {
        Ok(snapshots) => snapshots,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };

    match serde_json::to_value(SnapshotsListResult { snapshots }) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_apply_snapshot(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: ApplySnapshotParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let store = match SnapshotStore::open(&state.home, &params.project) {
        Ok(store) => store,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let snapshot = match store.get_snapshot(&params.snapshot_id) {
        Ok(snapshot) => snapshot,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let target = GitRepo::new(params.repo);
    let target_workspace_id = registered_workspace_id(state, &params.project, target.path());
    if let Err(err) = state.events.publish(SnapshotApplyStartedEvent {
        project_id: params.project.clone(),
        snapshot_id: snapshot.snapshot_id.clone(),
        target_workspace_id: target_workspace_id.clone(),
        dry_run: params.dry_run,
    }) {
        return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
    }
    let source = GitRepo::new(store.snapshot_repo_path());
    let result = if params.dry_run {
        match plan_apply_snapshot(&target, &source, &snapshot.metadata) {
            Ok(plan) => ApplySnapshotResult {
                snapshot,
                plan: Some(plan),
                verification: None,
            },
            Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
        }
    } else {
        match apply_snapshot(&target, &source, &snapshot.metadata) {
            Ok(verification) => {
                if let Err(err) = record_agent_snapshot_apply_audit(
                    state,
                    &snapshot,
                    target.path(),
                    target_workspace_id.as_deref(),
                    "apply.snapshot",
                    &verification,
                ) {
                    return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
                }
                if let Err(err) = state.events.publish(SnapshotApplyVerifiedEvent {
                    project_id: params.project.clone(),
                    snapshot_id: snapshot.snapshot_id.clone(),
                    target_workspace_id,
                    verification: verification.clone(),
                }) {
                    return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
                }
                ApplySnapshotResult {
                    snapshot,
                    plan: None,
                    verification: Some(verification),
                }
            }
            Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
        }
    };

    match serde_json::to_value(result) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_handoffs_list(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: HandoffsListParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let project_ids = match handoff_project_ids_for_query(state, params.project.as_deref()) {
        Ok(project_ids) => project_ids,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let mut handoffs = Vec::new();
    for project_id in project_ids {
        let db = match open_registered_project_db(state, &project_id) {
            Ok(db) => db,
            Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
        };
        let records = match db.list_handoffs(Some(&project_id)) {
            Ok(records) => records,
            Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
        };
        for record in records {
            let journal = if params.include_journal {
                match db.list_handoff_journal(&record.handoff_id) {
                    Ok(journal) => journal,
                    Err(err) => {
                        return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
                    }
                }
            } else {
                Vec::new()
            };
            handoffs.push(HandoffStatus { record, journal });
        }
    }

    match serde_json::to_value(HandoffsListResult { handoffs }) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_handoff_begin(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: HandoffBeginParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let source_device_id = match state.config.lock() {
        Ok(config) => config.device_id.clone(),
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let ttl_seconds = params
        .ttl_seconds
        .unwrap_or(DEFAULT_HANDOFF_TTL_SECONDS)
        .clamp(1, MAX_HANDOFF_TTL_SECONDS);
    if let Err(err) = ensure_known_target_device(state, &params.target_device_id) {
        return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
    }
    let mut db = match open_registered_project_db(state, &params.project) {
        Ok(db) => db,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let handoff = match db.begin_handoff(
        &params.lease_id,
        &source_device_id,
        &params.target_device_id,
        &params.source_generation,
        ttl_seconds,
    ) {
        Ok(handoff) => handoff,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let result = match handoff_mutation_result(&db, handoff) {
        Ok(result) => result,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };

    match serde_json::to_value(result) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_handoff_target_verify(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    handle_handoff_id_mutation(id, params, state, |db, handoff_id| {
        db.mark_handoff_target_verified(handoff_id)
    })
}

#[cfg(unix)]
fn handle_handoff_source_ready(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    handle_handoff_id_mutation(id, params, state, |db, handoff_id| {
        db.mark_handoff_source_ready(handoff_id)
    })
}

#[cfg(unix)]
fn handle_handoff_abort(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    handle_handoff_id_mutation(id, params, state, |db, handoff_id| {
        db.abort_handoff(handoff_id)
    })
}

#[cfg(unix)]
fn handle_handoff_commit(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: HandoffCommitParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let mut db = match open_registered_project_db(state, &params.project) {
        Ok(db) => db,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let handoff = match db.commit_handoff(
        &params.handoff_id,
        &params.observed_source_generation,
        unix_seconds(),
    ) {
        Ok(handoff) => handoff,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let result = match handoff_mutation_result(&db, handoff) {
        Ok(result) => result,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };

    match serde_json::to_value(result) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_handoff_recover(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: HandoffRecoverParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let mut db = match open_registered_project_db(state, &params.project) {
        Ok(db) => db,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let outcome = match db.recover_handoff(
        &params.handoff_id,
        &params.observed_source_generation,
        unix_seconds(),
    ) {
        Ok(outcome) => outcome,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let handoff = match db.get_handoff(&params.handoff_id) {
        Ok(Some(handoff)) => handoff,
        Ok(None) => {
            return RpcResponse::error(
                Some(id),
                RpcError::internal(format!("handoff {} disappeared", params.handoff_id)),
            );
        }
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let journal = match db.list_handoff_journal(&params.handoff_id) {
        Ok(journal) => journal,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let result = HandoffRecoverResult {
        outcome,
        handoff,
        journal,
    };

    match serde_json::to_value(result) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_handoff_id_mutation(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
    mutate: impl FnOnce(&mut MetadataDb, &str) -> devrelay_core::Result<HandoffRecord>,
) -> RpcResponse {
    let params: HandoffIdParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let mut db = match open_registered_project_db(state, &params.project) {
        Ok(db) => db,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let handoff = match mutate(&mut db, &params.handoff_id) {
        Ok(handoff) => handoff,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let result = match handoff_mutation_result(&db, handoff) {
        Ok(result) => result,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };

    match serde_json::to_value(result) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handoff_project_ids_for_query(
    state: &AgentState,
    project: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    let config = state
        .config
        .lock()
        .map_err(|err| anyhow::anyhow!("config lock poisoned: {err}"))?;
    if let Some(project) = project {
        if !config.project_registry.projects.contains_key(project) {
            anyhow::bail!("unknown project {project}");
        }
        return Ok(vec![project.to_string()]);
    }
    Ok(config.project_registry.projects.keys().cloned().collect())
}

#[cfg(unix)]
fn open_registered_project_db(state: &AgentState, project_id: &str) -> anyhow::Result<MetadataDb> {
    let is_registered = state
        .config
        .lock()
        .map_err(|err| anyhow::anyhow!("config lock poisoned: {err}"))?
        .project_registry
        .projects
        .contains_key(project_id);
    if !is_registered {
        anyhow::bail!("unknown project {project_id}");
    }
    Ok(MetadataDb::open(state.home.metadata_db_path(project_id))?)
}

#[cfg(unix)]
fn ensure_known_target_device(state: &AgentState, target_device_id: &str) -> anyhow::Result<()> {
    let db = MetadataDb::open(&state.database_path)?;
    if db.get_device(target_device_id)?.is_none() {
        anyhow::bail!("unknown target device {target_device_id}");
    }
    db.ensure_device_not_revoked(target_device_id, "begin handoff")?;
    Ok(())
}

#[cfg(unix)]
fn handoff_mutation_result(
    db: &MetadataDb,
    handoff: HandoffRecord,
) -> anyhow::Result<HandoffMutationResult> {
    let journal = db.list_handoff_journal(&handoff.handoff_id)?;
    Ok(HandoffMutationResult { handoff, journal })
}

#[cfg(unix)]
fn handle_recover_list(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: RecoverListParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let config = match state.config.lock() {
        Ok(config) => config,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let snapshots = match recover_list_snapshots(&state.home, &config, params.project.as_deref()) {
        Ok(snapshots) => snapshots,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };

    match serde_json::to_value(RecoverListResult { snapshots }) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_recover_show(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: RecoverShowParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let config = match state.config.lock() {
        Ok(config) => config,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let (_, _, snapshot) = match find_recovery_snapshot(
        &state.home,
        &config,
        params.project.as_deref(),
        &params.snapshot_id,
    ) {
        Ok(found) => found,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };

    match serde_json::to_value(RecoverShowResult { snapshot }) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_recover_open(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: RecoverOpenParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let (project_entry, store, snapshot) = {
        let config = match state.config.lock() {
            Ok(config) => config,
            Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
        };
        match find_recovery_snapshot(
            &state.home,
            &config,
            params.project.as_deref(),
            &params.snapshot_id,
        ) {
            Ok(found) => found,
            Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
        }
    };
    let source_path = match recovery_source_path(&project_entry) {
        Ok(path) => path,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let target = match prepare_recovery_workspace(&params.path, &source_path, &state.logger) {
        Ok(target) => target,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let snapshot_source = GitRepo::new(store.snapshot_repo_path());
    let verification = match apply_snapshot(&target, &snapshot_source, &snapshot.metadata) {
        Ok(verification) => verification,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    if let Err(err) = record_agent_snapshot_apply_audit(
        state,
        &snapshot,
        target.path(),
        None,
        "recover.open",
        &verification,
    ) {
        return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
    }
    let registered = if params.register {
        let mut config = match state.config.lock() {
            Ok(config) => config,
            Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
        };
        let workspace = match register_recovery_workspace(
            &mut config,
            &project_entry.project_id,
            target.path(),
        ) {
            Ok(workspace) => workspace,
            Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
        };
        if let Err(err) = config.save(&state.config_path) {
            return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
        }
        Some(workspace)
    } else {
        None
    };
    if let Some(workspace) = &registered
        && let Err(err) = publish_workspace_state_changed(state, workspace, None)
    {
        return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
    }
    let result = RecoverOpenResult {
        recovered: snapshot,
        path: target.path().to_path_buf(),
        name: params.name,
        registered,
        verification,
    };

    match serde_json::to_value(result) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_diagnostics_export(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: DiagnosticsExportParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let path = params.out.unwrap_or_else(|| {
        state
            .home
            .diagnostics_dir()
            .join(format!("diagnostics-{}.json", unix_seconds()))
    });
    let started_at_unix_millis = unix_millis();
    let (config, redactor, project_ids) = match state.config.lock() {
        Ok(config) => {
            let redactor = if params.include_sensitive_paths {
                LogRedactor::new()
            } else {
                LogRedactor::for_diagnostics(diagnostic_local_paths(&state.home, &config))
            };
            let project_ids = config
                .project_registry
                .projects
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            let config = if params.include_sensitive_paths {
                serde_json::to_value(&*config)
            } else {
                serde_json::to_value(config.redacted_for_diagnostics())
            };
            match config {
                Ok(value) => (value, redactor, project_ids),
                Err(err) => {
                    return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
                }
            }
        }
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let recent_logs = match recent_structured_logs(&state.home, &redactor, 50) {
        Ok(logs) => logs,
        Err(err) => vec![format!("failed to read recent logs: {err}")],
    };
    let state_machine_records = diagnostic_state_machine_records(&state.home, &project_ids);
    let git_command_exit_codes = match recent_git_command_exit_codes(&state.home, &redactor, 100) {
        Ok(exit_codes) => exit_codes,
        Err(err) => vec![serde_json::json!({
            "error": format!("failed to read Git command exit codes: {err}")
        })],
    };
    let health = match serde_json::to_value(state.health()) {
        Ok(health) => {
            if params.include_sensitive_paths {
                health
            } else {
                redact_json_value(health, &redactor)
            }
        }
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let finished_at_unix_millis = unix_millis();
    let bundle = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_version": RPC_PROTOCOL_VERSION,
        "capabilities": {
            "methods": AgentState::supported_methods(),
            "event_stream": true,
            "structured_logs": true,
        },
        "generated_at_unix_seconds": unix_seconds(),
        "timing": {
            "started_at_unix_millis": started_at_unix_millis,
            "finished_at_unix_millis": finished_at_unix_millis,
            "duration_millis": finished_at_unix_millis.saturating_sub(started_at_unix_millis),
        },
        "health": health,
        "config": config,
        "methods": AgentState::supported_methods(),
        "recent_structured_logs": recent_logs,
        "state_machine_records": state_machine_records,
        "git_command_exit_codes": git_command_exit_codes,
        "include_sensitive_paths": params.include_sensitive_paths,
        "source_code_included": false,
        "snapshot_objects_included": false,
    });

    if let Some(parent) = path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
    }
    let bytes = match serde_json::to_vec_pretty(&bundle) {
        Ok(bytes) => bytes,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    if let Err(err) = std::fs::write(&path, bytes) {
        return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
    }
    let result = DiagnosticsExportResult {
        path,
        include_sensitive_paths: params.include_sensitive_paths,
        source_code_included: false,
        snapshot_objects_included: false,
    };

    match serde_json::to_value(result) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_devices_list(id: devrelay_core::RpcId, state: &AgentState) -> RpcResponse {
    let db = match MetadataDb::open(&state.database_path) {
        Ok(db) => db,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let devices = match db.list_devices() {
        Ok(devices) => devices,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    match serde_json::to_value(DevicesListResult { devices }) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_activity_list(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: ActivityListParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let limit = params.limit.unwrap_or(100).clamp(1, 1_000);
    let mut events = Vec::new();

    match MetadataDb::open(&state.database_path)
        .and_then(|db| db.list_audit_events(params.project.as_deref(), limit))
    {
        Ok(mut records) => events.append(&mut records),
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }

    for project_id in project_ids_for_query(state, params.project.as_deref()) {
        let db_path = state.home.metadata_db_path(&project_id);
        if !db_path.exists() {
            continue;
        }
        match MetadataDb::open(&db_path)
            .and_then(|db| db.list_audit_events(Some(&project_id), limit))
        {
            Ok(mut records) => events.append(&mut records),
            Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
        }
    }

    events.sort_by(|left, right| {
        right
            .created_at_unix_seconds
            .cmp(&left.created_at_unix_seconds)
            .then(right.audit_id.cmp(&left.audit_id))
    });
    events.truncate(limit);

    match serde_json::to_value(ActivityListResult { events }) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_runs_list(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: RunsListParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let limit = params.limit.unwrap_or(100).clamp(1, 1_000);
    let mut runs = Vec::new();

    for project_id in project_ids_for_query(state, params.project.as_deref()) {
        let db_path = state.home.metadata_db_path(&project_id);
        if !db_path.exists() {
            continue;
        }
        match MetadataDb::open(&db_path).and_then(|db| db.list_task_runs(Some(&project_id), limit))
        {
            Ok(mut records) => runs.append(&mut records),
            Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
        }
    }

    runs.sort_by(|left, right| {
        right
            .updated_at_unix_seconds
            .cmp(&left.updated_at_unix_seconds)
            .then(
                right
                    .created_at_unix_seconds
                    .cmp(&left.created_at_unix_seconds),
            )
            .then(right.task_run_id.cmp(&left.task_run_id))
    });
    runs.truncate(limit);

    match serde_json::to_value(RunsListResult { runs }) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_settings_get(id: devrelay_core::RpcId, state: &AgentState) -> RpcResponse {
    let settings = match state.config.lock() {
        Ok(config) => settings_from_config(&config),
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    match serde_json::to_value(settings) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_settings_update(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: SettingsUpdateParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let settings = match state.config.lock() {
        Ok(mut config) => {
            if let Some(profile) = params.resource_profile {
                config.resource_profile = profile;
            }
            if let Some(enabled) = params.mdns_enabled {
                config.mdns_enabled = enabled;
            }
            if let Some(command) = params.editor_command {
                if command.trim().is_empty() {
                    return RpcResponse::error(
                        Some(id),
                        RpcError::invalid_params("editor_command must not be empty"),
                    );
                }
                config.editor.command = command;
            }
            if let Err(err) = config.save(&state.config_path) {
                return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
            }
            settings_from_config(&config)
        }
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };

    match serde_json::to_value(SettingsUpdateResult { settings }) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn settings_from_config(config: &LocalConfig) -> SettingsGetResult {
    SettingsGetResult {
        fabric_name: config.fabric_name.clone(),
        device_id: config.device_id.clone(),
        device_name: config.device_name.clone(),
        platform_key: config.platform_key.clone(),
        architecture: config.architecture.clone(),
        resource_profile: config.resource_profile,
        anchor_mode: config.anchor_mode,
        mdns_enabled: config.mdns_enabled,
        editor_command: config.editor.command.clone(),
        project_count: config.project_registry.projects.len(),
    }
}

#[cfg(unix)]
fn project_ids_for_query(state: &AgentState, project: Option<&str>) -> Vec<String> {
    if let Some(project) = project {
        return vec![project.to_string()];
    }
    state
        .config
        .lock()
        .map(|config| config.project_registry.projects.keys().cloned().collect())
        .unwrap_or_default()
}

#[cfg(unix)]
fn diagnostic_local_paths(home: &DevRelayHome, config: &LocalConfig) -> Vec<PathBuf> {
    let mut paths = vec![home.root().to_path_buf(), home.agent_socket_path()];
    for project in config.project_registry.projects.values() {
        paths.push(project.local_path.clone());
        if let Some(manifest_path) = &project.manifest_path {
            paths.push(manifest_path.clone());
        }
        for workspace in project.workspaces.values() {
            paths.push(workspace.local_path.clone());
        }
    }
    paths
}

#[cfg(unix)]
fn recent_structured_logs(
    home: &DevRelayHome,
    redactor: &LogRedactor,
    max_lines: usize,
) -> anyhow::Result<Vec<String>> {
    Ok(recent_log_lines(home, max_lines)?
        .into_iter()
        .map(|line| redactor.redact_text(&line))
        .collect())
}

#[cfg(unix)]
fn recent_log_lines(home: &DevRelayHome, max_lines: usize) -> anyhow::Result<Vec<String>> {
    let path = home.log_dir().join("agent.log");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let mut lines = raw
        .lines()
        .rev()
        .take(max_lines)
        .map(str::to_string)
        .collect::<Vec<_>>();
    lines.reverse();
    Ok(lines)
}

#[cfg(unix)]
fn diagnostic_state_machine_records(
    home: &DevRelayHome,
    project_ids: &[String],
) -> serde_json::Value {
    let mut sessions = Vec::new();
    let mut leases = Vec::new();
    let mut handoffs = Vec::new();
    let mut errors = Vec::new();

    for project_id in project_ids {
        let db_path = home.metadata_db_path(project_id);
        if !db_path.exists() {
            errors.push(format!("project {project_id}: metadata DB does not exist"));
            continue;
        }
        let db = match MetadataDb::open(&db_path) {
            Ok(db) => db,
            Err(err) => {
                errors.push(format!(
                    "project {project_id}: failed to open metadata DB: {err}"
                ));
                continue;
            }
        };

        match db.list_sessions(Some(project_id)) {
            Ok(records) => sessions.extend(records),
            Err(err) => errors.push(format!(
                "project {project_id}: failed to list sessions: {err}"
            )),
        }
        match db.list_leases(Some(project_id)) {
            Ok(records) => leases.extend(records),
            Err(err) => errors.push(format!(
                "project {project_id}: failed to list leases: {err}"
            )),
        }
        match db.list_handoffs(Some(project_id)) {
            Ok(records) => {
                for handoff in records {
                    let journal = match db.list_handoff_journal(&handoff.handoff_id) {
                        Ok(journal) => journal,
                        Err(err) => {
                            errors.push(format!(
                                "project {project_id}: failed to list handoff journal {}: {err}",
                                handoff.handoff_id
                            ));
                            Vec::new()
                        }
                    };
                    handoffs.push(serde_json::json!({
                        "record": handoff,
                        "journal": journal,
                    }));
                }
            }
            Err(err) => errors.push(format!(
                "project {project_id}: failed to list handoffs: {err}"
            )),
        }
    }

    serde_json::json!({
        "sessions": sessions,
        "leases": leases,
        "handoffs": handoffs,
        "errors": errors,
    })
}

#[cfg(unix)]
fn recent_git_command_exit_codes(
    home: &DevRelayHome,
    redactor: &LogRedactor,
    max_lines: usize,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let mut entries = Vec::new();
    for line in recent_log_lines(home, max_lines)? {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let Some(fields) = value.get("fields").and_then(|fields| fields.as_object()) else {
            continue;
        };
        if fields.get("command").and_then(|command| command.as_str()) != Some("git") {
            continue;
        }
        let Some(exit_code) = fields.get("exit_code").and_then(|code| code.as_str()) else {
            continue;
        };

        let mut entry = serde_json::Map::new();
        entry.insert(
            "timestamp_unix_millis".to_string(),
            value
                .get("timestamp_unix_millis")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        );
        entry.insert(
            "target".to_string(),
            value
                .get("target")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        );
        entry.insert(
            "operation_id".to_string(),
            value
                .get("operation_id")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        );
        entry.insert("command".to_string(), serde_json::json!("git"));
        if let Some(args) = fields.get("args") {
            entry.insert("args".to_string(), args.clone());
        }
        if let Some(success) = fields.get("success").and_then(|success| success.as_str()) {
            let success = match success {
                "true" => serde_json::json!(true),
                "false" => serde_json::json!(false),
                value => serde_json::json!(value),
            };
            entry.insert("success".to_string(), success);
        }
        entry.insert(
            "exit_code".to_string(),
            exit_code
                .parse::<i64>()
                .map(serde_json::Value::from)
                .unwrap_or_else(|_| serde_json::json!(exit_code)),
        );

        entries.push(redact_json_value(
            serde_json::Value::Object(entry),
            redactor,
        ));
    }
    Ok(entries)
}

#[cfg(unix)]
fn redact_json_value(value: serde_json::Value, redactor: &LogRedactor) -> serde_json::Value {
    match value {
        serde_json::Value::String(value) => serde_json::Value::String(redactor.redact_text(&value)),
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .into_iter()
                .map(|value| redact_json_value(value, redactor))
                .collect(),
        ),
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, redact_json_value(value, redactor)))
                .collect(),
        ),
        value => value,
    }
}

#[cfg(unix)]
fn handle_projects_add(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: ProjectsAddParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let device_id = match state.config.lock() {
        Ok(config) => config.device_id.clone(),
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let entry =
        match build_project_registry_entry(&params.path, params.manifest.as_deref(), &device_id) {
            Ok(entry) => entry,
            Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
        };

    let mut config = match state.config.lock() {
        Ok(config) => config,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    if let Err(err) = ensure_workspace_not_registered(&config, &entry.local_path) {
        return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
    }
    if AgentRole::from_anchor_mode(config.anchor_mode) == AgentRole::Anchor
        && let Err(err) = AnchorSnapshotRepo::open(&state.home, &entry.project_id)
    {
        return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
    }
    merge_project_registry_entry(&mut config, entry.clone());
    if let Err(err) = config.save(&state.config_path) {
        return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
    }
    if let Err(err) = ensure_default_session_for_project(&state.home, &entry) {
        return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
    }
    for workspace in entry.workspaces.values() {
        if let Err(err) = publish_workspace_state_changed(state, workspace, None) {
            return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
        }
    }

    match serde_json::to_value(ProjectResult { project: entry }) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_projects_list(id: devrelay_core::RpcId, state: &AgentState) -> RpcResponse {
    let config = match state.config.lock() {
        Ok(config) => config,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let projects = config
        .project_registry
        .projects
        .values()
        .cloned()
        .collect::<Vec<_>>();

    match serde_json::to_value(ProjectsListResult { projects }) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_projects_show(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: ProjectsShowParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let config = match state.config.lock() {
        Ok(config) => config,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let project = match find_project(&config, &params.id_or_name) {
        Some(project) => project.clone(),
        None => {
            return RpcResponse::error(
                Some(id),
                RpcError::internal(format!("unknown project {}", params.id_or_name)),
            );
        }
    };

    match serde_json::to_value(ProjectResult { project }) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn handle_projects_remove(
    id: devrelay_core::RpcId,
    params: serde_json::Value,
    state: &AgentState,
) -> RpcResponse {
    let params: ProjectsRemoveParams = match serde_json::from_value(params) {
        Ok(params) => params,
        Err(err) => return RpcResponse::error(Some(id), RpcError::invalid_params(err.to_string())),
    };
    let mut config = match state.config.lock() {
        Ok(config) => config,
        Err(err) => return RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    };
    let project_id = match find_project(&config, &params.id_or_name) {
        Some(project) => project.project_id.clone(),
        None => {
            return RpcResponse::error(
                Some(id),
                RpcError::internal(format!("unknown project {}", params.id_or_name)),
            );
        }
    };
    let project = match config.project_registry.projects.remove(&project_id) {
        Some(project) => project,
        None => {
            return RpcResponse::error(
                Some(id),
                RpcError::internal(format!("project disappeared {project_id}")),
            );
        }
    };
    if let Err(err) = config.save(&state.config_path) {
        return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
    }

    match serde_json::to_value(ProjectResult { project }) {
        Ok(result) => RpcResponse::success(id, result),
        Err(err) => RpcResponse::error(Some(id), RpcError::internal(err.to_string())),
    }
}

#[cfg(unix)]
fn recover_list_snapshots(
    home: &DevRelayHome,
    config: &LocalConfig,
    project: Option<&str>,
) -> anyhow::Result<Vec<StoredSnapshot>> {
    if let Some(project) = project {
        let entry = find_project(config, project)
            .ok_or_else(|| anyhow::anyhow!("unknown project {project}"))?;
        let store = SnapshotStore::open(home, &entry.project_id)?;
        return Ok(store.list_snapshots()?);
    }

    let mut snapshots = Vec::new();
    for project in config.project_registry.projects.values() {
        let store = SnapshotStore::open(home, &project.project_id)?;
        snapshots.extend(store.list_snapshots()?);
    }
    snapshots.sort_by(|left, right| {
        left.project_id
            .cmp(&right.project_id)
            .then(left.sequence_number.cmp(&right.sequence_number))
    });
    Ok(snapshots)
}

#[cfg(unix)]
fn find_recovery_snapshot(
    home: &DevRelayHome,
    config: &LocalConfig,
    project: Option<&str>,
    snapshot_id: &str,
) -> anyhow::Result<(ProjectRegistryEntry, SnapshotStore, StoredSnapshot)> {
    if let Some(project) = project {
        let entry = find_project(config, project)
            .ok_or_else(|| anyhow::anyhow!("unknown project {project}"))?
            .clone();
        let store = SnapshotStore::open(home, &entry.project_id)?;
        let snapshot = store.get_snapshot(snapshot_id)?;
        return Ok((entry, store, snapshot));
    }

    for project in config.project_registry.projects.values() {
        let store = SnapshotStore::open(home, &project.project_id)?;
        if let Ok(snapshot) = store.get_snapshot(snapshot_id) {
            return Ok((project.clone(), store, snapshot));
        }
    }

    Err(anyhow::anyhow!("unknown snapshot {snapshot_id}"))
}

#[cfg(unix)]
fn recovery_source_path(project: &ProjectRegistryEntry) -> anyhow::Result<PathBuf> {
    if let Some(workspace) = project.workspaces.values().find(|workspace| {
        workspace.local_path.exists() && workspace.state == WorkspaceState::Active
    }) {
        return Ok(workspace.local_path.clone());
    }
    if project.local_path.exists() {
        return Ok(project.local_path.clone());
    }
    Err(anyhow::anyhow!(
        "no existing source workspace for project {}",
        project.project_id
    ))
}

#[cfg(unix)]
fn prepare_recovery_workspace(
    path: &Path,
    source_path: &Path,
    logger: &AgentLogger,
) -> anyhow::Result<GitRepo> {
    if path.join(".git").exists() {
        let target = GitRepo::new(path);
        let status = target.status()?;
        if !status.is_clean() {
            return Err(anyhow::anyhow!(
                "target workspace is dirty: {}",
                status.short_summary()
            ));
        }
        return Ok(target);
    }

    if path.exists() && std::fs::read_dir(path)?.next().is_some() {
        return Err(anyhow::anyhow!(
            "{} exists and is not an empty recovery directory",
            path.display()
        ));
    }

    std::fs::create_dir_all(path)?;
    clone_repository(source_path, path, logger)?;
    Ok(GitRepo::new(path))
}

#[cfg(unix)]
fn clone_repository(
    source_path: &Path,
    target_path: &Path,
    logger: &AgentLogger,
) -> anyhow::Result<()> {
    let output = std::process::Command::new("git")
        .arg("clone")
        .arg(source_path)
        .arg(target_path)
        .output()?;
    let exit_code = output
        .status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_string());
    logger.log(
        StructuredLogRecord::new(
            StructuredLogLevel::Info,
            "agent.git",
            "Git command completed",
        )
        .with_field("command", "git")
        .with_field(
            "args",
            format!("clone {} {}", source_path.display(), target_path.display()),
        )
        .with_field("exit_code", exit_code)
        .with_field("success", output.status.success().to_string()),
    );
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "git clone {} {} failed: {}",
            source_path.display(),
            target_path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn register_recovery_workspace(
    config: &mut LocalConfig,
    project_id: &str,
    path: &Path,
) -> anyhow::Result<WorkspaceRegistryEntry> {
    let root = resolve_git_root(path)?;
    ensure_workspace_not_registered(config, &root)?;
    let workspace_id = workspace_id_for(project_id, &config.device_id, &root);
    let repo = GitRepo::new(&root);
    let workspace = WorkspaceRegistryEntry {
        workspace_id: workspace_id.clone(),
        project_id: project_id.to_string(),
        device_id: config.device_id.clone(),
        local_path: root,
        platform_profile: current_platform_profile(),
        state: WorkspaceState::Active,
        last_seen_head: head_oid(&repo),
        last_checkpoint_id: None,
    };
    let project = config
        .project_registry
        .projects
        .get_mut(project_id)
        .ok_or_else(|| anyhow::anyhow!("unknown project {project_id}"))?;
    project.workspaces.insert(workspace_id, workspace.clone());
    Ok(workspace)
}

#[cfg(unix)]
fn publish_workspace_state_changed(
    state: &AgentState,
    workspace: &WorkspaceRegistryEntry,
    previous_state: Option<WorkspaceState>,
) -> anyhow::Result<()> {
    state.events.publish(WorkspaceStateChangedEvent {
        project_id: workspace.project_id.clone(),
        workspace_id: workspace.workspace_id.clone(),
        previous_state,
        state: workspace.state,
        device_id: Some(workspace.device_id.clone()),
        last_seen_head: workspace.last_seen_head.clone(),
        last_checkpoint_id: workspace.last_checkpoint_id.clone(),
    })?;
    Ok(())
}

#[cfg(unix)]
fn registered_workspace_id(state: &AgentState, project_id: &str, path: &Path) -> Option<String> {
    let config = state.config.lock().ok()?;
    config
        .project_registry
        .projects
        .get(project_id)?
        .workspaces
        .values()
        .find(|workspace| workspace.local_path == path)
        .map(|workspace| workspace.workspace_id.clone())
}

#[cfg(unix)]
fn record_agent_snapshot_apply_audit(
    state: &AgentState,
    snapshot: &StoredSnapshot,
    target_path: &Path,
    target_workspace_id: Option<&str>,
    operation: &str,
    verification: &devrelay_core::VerificationDetails,
) -> anyhow::Result<()> {
    let actor_device_id = state
        .config
        .lock()
        .ok()
        .map(|config| config.device_id.clone());
    let db = MetadataDb::open(state.home.metadata_db_path(&snapshot.project_id))?;
    let mut audit = AuditEventInput::new(
        AuditEventType::SnapshotApplied,
        AuditOutcome::Succeeded,
        "snapshot applied to workspace",
    )
    .with_detail(serde_json::json!({
        "operation": operation,
        "target_path": target_path.to_string_lossy().to_string(),
        "target_workspace_id": target_workspace_id,
        "verified_state_hash": verification.state_hash.as_str(),
        "included_untracked_count": verification.included_untracked.len(),
        "excluded_path_count": verification.excluded_paths.len(),
    }));
    audit.project_id = Some(snapshot.project_id.clone());
    audit.actor_device_id = actor_device_id;
    audit.session_id = snapshot.session_id.clone();
    audit.snapshot_id = Some(snapshot.snapshot_id.clone());
    db.record_audit_event(audit)?;
    Ok(())
}

#[cfg(unix)]
fn ensure_default_session_for_project(
    home: &DevRelayHome,
    project: &ProjectRegistryEntry,
) -> anyhow::Result<()> {
    let db = MetadataDb::open(home.metadata_db_path(&project.project_id))?;
    db.ensure_default_session(&project.project_id, &project.display_name, None)?;
    Ok(())
}

#[cfg(unix)]
fn build_project_registry_entry(
    path: &Path,
    manifest_path: Option<&Path>,
    device_id: &str,
) -> anyhow::Result<ProjectRegistryEntry> {
    let root = resolve_git_root(path)?;
    let manifest_path = manifest_path.map(PathBuf::from).or_else(|| {
        root.join("devrelay.toml")
            .exists()
            .then(|| root.join("devrelay.toml"))
    });
    let manifest = manifest_path
        .as_ref()
        .map(Manifest::load)
        .transpose()
        .with_context(|| "failed to load project manifest")?;
    let project_id = manifest
        .as_ref()
        .map(|manifest| manifest.project_id.clone())
        .unwrap_or_else(|| generated_project_id(&root));
    let display_name = manifest
        .as_ref()
        .map(|manifest| manifest.name.clone())
        .or_else(|| {
            root.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| project_id.clone());
    let repo = GitRepo::new(&root);
    let workspace_id = workspace_id_for(&project_id, device_id, &root);
    let workspace = WorkspaceRegistryEntry {
        workspace_id: workspace_id.clone(),
        project_id: project_id.clone(),
        device_id: device_id.to_string(),
        local_path: root.clone(),
        platform_profile: current_platform_profile(),
        state: WorkspaceState::Active,
        last_seen_head: head_oid(&repo),
        last_checkpoint_id: None,
    };

    Ok(ProjectRegistryEntry {
        project_id,
        display_name,
        local_path: root,
        workspaces: BTreeMap::from([(workspace_id, workspace)]),
        manifest_path,
        remote_url_fingerprint: remote_fingerprint(&repo),
        root_commit_fingerprint: root_commit_fingerprint(&repo),
    })
}

#[cfg(unix)]
fn merge_project_registry_entry(config: &mut LocalConfig, entry: ProjectRegistryEntry) {
    let project_id = entry.project_id.clone();
    if let Some(existing) = config.project_registry.projects.get_mut(&project_id) {
        for (workspace_id, workspace) in entry.workspaces {
            existing.workspaces.insert(workspace_id, workspace);
        }
        if existing.manifest_path.is_none() {
            existing.manifest_path = entry.manifest_path;
        }
        if existing.remote_url_fingerprint.is_none() {
            existing.remote_url_fingerprint = entry.remote_url_fingerprint;
        }
        if existing.root_commit_fingerprint.is_none() {
            existing.root_commit_fingerprint = entry.root_commit_fingerprint;
        }
    } else {
        config.project_registry.projects.insert(project_id, entry);
    }
}

#[cfg(unix)]
fn ensure_workspace_not_registered(config: &LocalConfig, local_path: &Path) -> anyhow::Result<()> {
    if let Some((project, workspace)) = config.project_registry.workspace_by_path(local_path) {
        return Err(anyhow::anyhow!(
            "{} is already registered as workspace {} for {}",
            local_path.display(),
            workspace.workspace_id,
            project.project_id
        ));
    }
    for project in config.project_registry.projects.values() {
        if project.workspaces.is_empty() && project.local_path == local_path {
            return Err(anyhow::anyhow!(
                "{} is already registered as {}",
                local_path.display(),
                project.project_id
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn resolve_git_root(path: &Path) -> anyhow::Result<PathBuf> {
    let repo = GitRepo::new(path);
    let raw = repo
        .run(&["rev-parse", "--show-toplevel"])
        .map_err(|_| anyhow::anyhow!("path is not a Git repository: {}", path.display()))?;
    Ok(PathBuf::from(raw))
}

#[cfg(unix)]
fn find_project<'a>(config: &'a LocalConfig, id_or_name: &str) -> Option<&'a ProjectRegistryEntry> {
    config
        .project_registry
        .projects
        .get(id_or_name)
        .or_else(|| {
            config
                .project_registry
                .projects
                .values()
                .find(|project| project.display_name == id_or_name)
        })
}

#[cfg(unix)]
fn generated_project_id(root: &Path) -> String {
    format!("p_{}", hash_text(&root.to_string_lossy()))
}

#[cfg(unix)]
fn remote_fingerprint(repo: &GitRepo) -> Option<String> {
    repo.run(&["remote", "get-url", "origin"])
        .ok()
        .map(|remote| format!("remote_{}", hash_text(remote.trim())))
}

#[cfg(unix)]
fn root_commit_fingerprint(repo: &GitRepo) -> Option<String> {
    repo.run(&["rev-list", "--max-parents=0", "HEAD"])
        .ok()
        .and_then(|roots| roots.lines().next().map(str::to_string))
        .map(|root| format!("root_{}", hash_text(&root)))
}

#[cfg(unix)]
fn head_oid(repo: &GitRepo) -> Option<String> {
    repo.run(&["rev-parse", "--verify", "HEAD"])
        .ok()
        .map(|head| head.trim().to_string())
        .filter(|head| !head.is_empty())
}

#[cfg(unix)]
fn current_platform_profile() -> String {
    current_platform_key()
}

#[cfg(unix)]
fn hash_text(value: &str) -> String {
    let digest = blake3::hash(value.as_bytes());
    digest.to_hex()[..16].to_string()
}

#[cfg(unix)]
fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(unix)]
fn unix_millis() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX));
    millis as u64
}

fn install_shutdown_handler() -> anyhow::Result<Arc<AtomicBool>> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let signal_shutdown = Arc::clone(&shutdown);
    ctrlc::set_handler(move || {
        signal_shutdown.store(true, Ordering::SeqCst);
    })
    .context("failed to install shutdown handler")?;
    Ok(shutdown)
}

fn load_or_create_config(path: &PathBuf) -> anyhow::Result<LocalConfig> {
    if path.exists() {
        LocalConfig::load(path).with_context(|| format!("failed to load {}", path.display()))
    } else {
        let config = LocalConfig::new_for_local_device();
        config
            .save(path)
            .with_context(|| format!("failed to save {}", path.display()))?;
        Ok(config)
    }
}
