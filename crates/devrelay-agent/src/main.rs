use anyhow::Context;
use clap::{Parser, ValueEnum};
#[cfg(unix)]
use devrelay_core::{
    CheckpointCreateParams, CheckpointCreateResult, GitRepo, IpcConnection, IpcLimits,
    IpcTransport, Manifest, ProjectRegistryEntry, ProjectResult, ProjectsAddParams,
    ProjectsListResult, ProjectsShowParams, RPC_PROTOCOL_VERSION, RpcError, RpcRequest,
    RpcResponse, RpcVersionNegotiationParams, RpcVersionNegotiationResult, SnapshotStore,
    SnapshotsListParams, SnapshotsListResult, StatusGetParams, StatusGetResult, UnixIpcConnection,
    UnixIpcListener, WorkspaceRegistryEntry, WorkspaceState, classify_untracked_paths,
    workspace_id_for,
};
use devrelay_core::{
    DevRelayHome, LocalConfig, METHOD_AGENT_HEALTH, METHOD_CHECKPOINT_CREATE, METHOD_PROJECTS_ADD,
    METHOD_PROJECTS_LIST, METHOD_PROJECTS_SHOW, METHOD_RPC_NEGOTIATE, METHOD_SNAPSHOTS_LIST,
    METHOD_STATUS_GET, MetadataDb,
};
use serde::Serialize;
#[cfg(unix)]
use std::collections::BTreeMap;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

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

#[derive(Debug, Serialize)]
struct AgentHealth {
    status: &'static str,
    foreground: bool,
    config_path: PathBuf,
    socket_path: PathBuf,
    project_count: usize,
    database_path: PathBuf,
    shutdown_requested: bool,
}

#[derive(Clone)]
struct AgentState {
    foreground: bool,
    home: DevRelayHome,
    config_path: PathBuf,
    socket_path: PathBuf,
    config: Arc<Mutex<LocalConfig>>,
    database_path: PathBuf,
    shutdown: Arc<AtomicBool>,
}

impl AgentState {
    fn health(&self) -> AgentHealth {
        AgentHealth {
            status: "ok",
            foreground: self.foreground,
            config_path: self.config_path.clone(),
            socket_path: self.socket_path.clone(),
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

    fn supported_methods() -> Vec<String> {
        vec![
            METHOD_RPC_NEGOTIATE.to_string(),
            METHOD_AGENT_HEALTH.to_string(),
            METHOD_STATUS_GET.to_string(),
            METHOD_PROJECTS_ADD.to_string(),
            METHOD_PROJECTS_LIST.to_string(),
            METHOD_PROJECTS_SHOW.to_string(),
            METHOD_CHECKPOINT_CREATE.to_string(),
            METHOD_SNAPSHOTS_LIST.to_string(),
        ]
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let shutdown = install_shutdown_handler()?;
    let home = DevRelayHome::resolve()?;
    home.create_base_dirs()?;
    let config_path = cli.config.clone().unwrap_or_else(|| home.config_file());
    let config = load_or_create_config(&config_path)?;
    let database_path = home.root().join("agent.sqlite");
    let _db = MetadataDb::open(&database_path)?;
    let socket_path = cli
        .socket_path
        .clone()
        .unwrap_or_else(|| home.agent_socket_path());

    eprintln!(
        "devrelay-agent started foreground={} log_level={:?} projects={} socket={}",
        cli.foreground,
        cli.log_level,
        config.project_registry.projects.len(),
        socket_path.display()
    );

    let state = AgentState {
        foreground: cli.foreground,
        home,
        config_path,
        socket_path,
        config: Arc::new(Mutex::new(config)),
        database_path,
        shutdown: Arc::clone(&shutdown),
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
        eprintln!("devrelay-agent shutdown requested");
    }
    Ok(())
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
            Err(err) => eprintln!("devrelay-agent IPC accept error: {err}"),
        }
    }
}

#[cfg(unix)]
fn handle_rpc_connection(mut connection: UnixIpcConnection, state: AgentState) {
    let response = match connection.read_message(IpcLimits::default()) {
        Ok(bytes) => match RpcRequest::parse(&bytes) {
            Ok(request) => handle_rpc_request(request, &state),
            Err(error) => RpcResponse::error(None, error),
        },
        Err(err) => {
            eprintln!("devrelay-agent IPC read error: {err}");
            return;
        }
    };

    let bytes = match serde_json::to_vec(&response) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("devrelay-agent RPC response serialization error: {err}");
            return;
        }
    };
    if let Err(err) = connection.write_message(&bytes, IpcLimits::default()) {
        eprintln!("devrelay-agent IPC write error: {err}");
    }
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
        METHOD_CHECKPOINT_CREATE => handle_checkpoint_create(id, request.params, state),
        METHOD_SNAPSHOTS_LIST => handle_snapshots_list(id, request.params, state),
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
    merge_project_registry_entry(&mut config, entry.clone());
    if let Err(err) = config.save(&state.config_path) {
        return RpcResponse::error(Some(id), RpcError::internal(err.to_string()));
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
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

#[cfg(unix)]
fn hash_text(value: &str) -> String {
    let digest = blake3::hash(value.as_bytes());
    digest.to_hex()[..16].to_string()
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
        let config = LocalConfig::default();
        config
            .save(path)
            .with_context(|| format!("failed to save {}", path.display()))?;
        Ok(config)
    }
}
