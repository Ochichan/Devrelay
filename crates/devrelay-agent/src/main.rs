use anyhow::Context;
use clap::{Parser, ValueEnum};
use devrelay_core::{DevRelayHome, LocalConfig, MetadataDb};
#[cfg(unix)]
use devrelay_core::{
    GitRepo, IpcConnection, IpcLimits, IpcTransport, METHOD_AGENT_HEALTH, METHOD_RPC_NEGOTIATE,
    METHOD_STATUS_GET, Manifest, RPC_PROTOCOL_VERSION, RpcError, RpcRequest, RpcResponse,
    RpcVersionNegotiationParams, RpcVersionNegotiationResult, StatusGetParams, StatusGetResult,
    UnixIpcConnection, UnixIpcListener, classify_untracked_paths,
};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{
    Arc,
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
    config_path: PathBuf,
    socket_path: PathBuf,
    project_count: usize,
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
            project_count: self.project_count,
            database_path: self.database_path.clone(),
            shutdown_requested: self.shutdown.load(Ordering::SeqCst),
        }
    }

    fn supported_methods() -> Vec<String> {
        vec![
            METHOD_RPC_NEGOTIATE.to_string(),
            METHOD_AGENT_HEALTH.to_string(),
            METHOD_STATUS_GET.to_string(),
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
        config_path,
        socket_path,
        project_count: config.project_registry.projects.len(),
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
