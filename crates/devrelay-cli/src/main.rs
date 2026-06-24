//! Command-line interface for the local DevRelay foundation.
//!
//! This binary is intentionally thin in M0. It loads manifests, delegates Git
//! state work to `devrelay-core`, and renders human or JSON output for explicit
//! local commands.

use anyhow::{Context, Error};
use clap::{Parser, Subcommand, ValueEnum};
#[cfg(unix)]
use devrelay_core::AgentRpcClient;
use devrelay_core::{
    AgentRole, AnchorLayout, AnchorMode, AnchorSnapshotMaintenanceReport, AnchorSnapshotRepo,
    ApplySnapshotParams, ApplySnapshotResult, AuditEventInput, AuditEventRecord, AuditEventType,
    AuditOutcome, CheckpointCreateParams, CheckpointCreateResult, DevRelayError, DevRelayHome,
    DeviceIdentity, DevicePublicIdentity, DeviceRevocationRecord, DiagnosticsExportParams,
    DiagnosticsExportResult, DiscoveryAdvertisement, DiscoveryRole, DiscoveryService,
    EnvironmentDoctorOptions, EnvironmentDoctorReport, EnvironmentStatusEntry,
    EnvironmentStatusParams, EnvironmentStatusResult, ErrorInfo, FabricIdentityBundle,
    FabricIdentityStore, GitPerformanceDoctorReport, GitRepo, LineEndingDoctorReport, LocalConfig,
    LogRedactor, METHOD_APPLY_SNAPSHOT, METHOD_CHECKPOINT_CREATE, METHOD_DIAGNOSTICS_EXPORT,
    METHOD_ENVIRONMENT_STATUS, METHOD_METRICS_EXPORT, METHOD_PROJECTS_ADD, METHOD_PROJECTS_LIST,
    METHOD_PROJECTS_REMOVE, METHOD_PROJECTS_SHOW, METHOD_RECOVER_LIST, METHOD_RECOVER_OPEN,
    METHOD_RECOVER_SHOW, METHOD_STATUS_GET, Manifest, MetadataDb, MetricsExportParams,
    MetricsExportResult, PairingSession, PairingStartRequest, PathDecision,
    PathPortabilityDoctorReport, PatternConfig, PortablePathsPolicy, ProjectRegistryEntry,
    ProjectResult, ProjectsAddParams, ProjectsListResult, ProjectsRemoveParams, ProjectsShowParams,
    RecoverListParams, RecoverListResult, RecoverOpenParams, RecoverOpenResult, RecoverShowParams,
    RecoverShowResult, SecretProviderLocalConfig, SecretScannerConfig, ServiceTemplate,
    ServiceTemplateInput, ServiceTemplateKind, SnapshotMetadata, SnapshotStore, StatusGetParams,
    StatusGetResult, StatusSummary, StoredSession, StoredSnapshot, SystemEnvironmentCommandRunner,
    UntrackedPolicy, WorkspaceConfig, WorkspaceRegistryEntry, WorkspaceState,
    WslFilesystemDoctorReport, apply_snapshot, build_discovery_advertisement,
    classify_untracked_paths, collect_local_metrics_report, create_snapshot, current_platform_key,
    linux_systemd_user_template, load_hydration_state, macos_launch_agent_template,
    plan_apply_snapshot, read_snapshot_file, run_environment_doctor, run_git_performance_doctor,
    run_line_ending_doctor, run_path_portability_doctor, run_wsl_filesystem_doctor,
    workspace_id_for, write_snapshot_file,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(name = "devrelay")]
#[command(about = "DevRelay personal development fabric CLI")]
#[command(version)]
#[command(
    after_help = "Examples:\n  devrelay manifest check devrelay_spec_bundle/devrelay.toml\n  devrelay status --repo . --manifest devrelay.toml --json\n  devrelay checkpoint --repo . --manifest devrelay.toml --json\n  devrelay apply --repo ../target --source . --snapshot .devrelay/snapshots/<id>.json --dry-run"
)]
struct Cli {
    #[arg(long, global = true)]
    json_errors: bool,
    #[arg(long, global = true)]
    direct: bool,
    #[arg(long, global = true)]
    agent_socket: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    Audit {
        #[command(subcommand)]
        command: AuditCommand,
    },
    Anchor {
        #[command(subcommand)]
        command: AnchorCommand,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Diagnostics {
        #[command(subcommand)]
        command: DiagnosticsCommand,
    },
    Doctor {
        #[command(subcommand)]
        command: DoctorCommand,
    },
    Device {
        #[command(subcommand)]
        command: DeviceCommand,
    },
    Devices {
        #[command(subcommand)]
        command: DevicesCommand,
    },
    Discovery {
        #[command(subcommand)]
        command: DiscoveryCommand,
    },
    Environment {
        #[command(subcommand)]
        command: EnvironmentCommand,
    },
    Metrics {
        #[command(subcommand)]
        command: MetricsCommand,
    },
    Identity {
        #[command(subcommand)]
        command: IdentityCommand,
    },
    Continue {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        target: PathBuf,
        #[arg(long)]
        manifest: Option<PathBuf>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "block")]
        dirty_policy: DirtyPolicy,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    Manifest {
        #[command(subcommand)]
        command: ManifestCommand,
    },
    Pairing {
        #[command(subcommand)]
        command: PairingCommand,
    },
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    Projects {
        #[command(subcommand)]
        command: ProjectsCommand,
    },
    Recover {
        #[command(subcommand)]
        command: RecoverCommand,
    },
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    Sessions {
        #[command(subcommand)]
        command: SessionsCommand,
    },
    Snapshot {
        #[command(subcommand)]
        command: SnapshotCommand,
    },
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },
    Status {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        #[arg(long, default_value = "devrelay.toml")]
        manifest: PathBuf,
        #[arg(long)]
        json: bool,
    },
    Checkpoint {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        #[arg(long, default_value = "devrelay.toml")]
        manifest: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        label: Option<String>,
        #[arg(long)]
        pin: bool,
        #[arg(long)]
        json: bool,
    },
    Apply {
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        snapshot: PathBuf,
        #[arg(long)]
        dry_run: bool,
        #[arg(long, value_enum, default_value = "block")]
        dirty_policy: DirtyPolicy,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum AgentCommand {
    Install {
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        agent_bin: Option<PathBuf>,
        #[arg(long)]
        service_dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Uninstall {
        #[arg(long)]
        service_dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Status {
        #[arg(long)]
        service_dir: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum AuditCommand {
    List {
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        include_sensitive_paths: bool,
        #[arg(long)]
        json: bool,
    },
    Export {
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long, default_value_t = 1_000)]
        limit: usize,
        #[arg(long)]
        include_sensitive_paths: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum AnchorCommand {
    Init {
        #[arg(long)]
        json: bool,
    },
    Status {
        #[arg(long)]
        json: bool,
    },
    Maintenance {
        #[arg(long)]
        project: String,
        #[arg(long)]
        gc: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum DiagnosticsCommand {
    Export {
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        include_sensitive_paths: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum MetricsCommand {
    Export {
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        include_sensitive_paths: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum DoctorCommand {
    GitPerformance {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        #[arg(long)]
        fix_safe: bool,
        #[arg(long)]
        json: bool,
    },
    Paths {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        #[arg(long, default_value = "devrelay.toml")]
        manifest: PathBuf,
        #[arg(long)]
        target_platform: Option<String>,
        #[arg(long)]
        json: bool,
    },
    LineEndings {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        #[arg(long)]
        target_platform: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Environment {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        #[arg(long, default_value = "devrelay.toml")]
        manifest: PathBuf,
        #[arg(long)]
        platform_key: Option<String>,
        #[arg(long)]
        secrets_config: Option<PathBuf>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        run_healthcheck: bool,
        #[arg(long)]
        allow_devcontainer_prepare: bool,
        #[arg(long)]
        json: bool,
    },
    WslFilesystem {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        #[arg(long)]
        platform_key: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum DeviceCommand {
    Show {
        id: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Revoke {
        device_id: String,
        #[arg(long)]
        reason: String,
        #[arg(long)]
        key_rotation_required: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum DevicesCommand {
    List {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum DiscoveryCommand {
    Advertise {
        #[arg(long, value_enum)]
        role: DiscoveryRoleArg,
        #[arg(long)]
        port: u16,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
    Browse {
        #[arg(long, value_enum)]
        role: DiscoveryRoleArg,
        #[arg(long)]
        manual_address: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum DiscoveryRoleArg {
    Anchor,
    Peer,
}

impl From<DiscoveryRoleArg> for DiscoveryRole {
    fn from(value: DiscoveryRoleArg) -> Self {
        match value {
            DiscoveryRoleArg::Anchor => Self::Anchor,
            DiscoveryRoleArg::Peer => Self::Peer,
        }
    }
}

#[derive(Debug, Subcommand)]
enum EnvironmentCommand {
    Status {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum IdentityCommand {
    Init {
        #[arg(long)]
        json: bool,
    },
    Show {
        #[arg(long)]
        json: bool,
    },
    RecoveryExport {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum PairingCommand {
    Start {
        #[arg(long)]
        peer_device_id: String,
        #[arg(long)]
        peer_name: String,
        #[arg(long)]
        peer_signing_public_key: String,
        #[arg(long)]
        peer_network_public_key: String,
        #[arg(long)]
        peer_ephemeral_public_key: String,
        #[arg(long)]
        anchor: Option<String>,
        #[arg(long, default_value_t = 300)]
        ttl_seconds: u64,
        #[arg(long)]
        json: bool,
    },
    Confirm {
        pairing_id: String,
        #[arg(long)]
        code: String,
        #[arg(long, default_value_t = 31_536_000)]
        certificate_ttl_seconds: u64,
        #[arg(long)]
        json: bool,
    },
    Abort {
        pairing_id: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum SessionsCommand {
    List {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum SessionCommand {
    Show {
        session_id: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Fork {
        session_id: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        name: String,
        #[arg(long)]
        json: bool,
    },
    Archive {
        session_id: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum DirtyPolicy {
    Block,
    SnapshotAndFork,
    NewWorkspace,
}

impl DirtyPolicy {
    fn label(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::SnapshotAndFork => "snapshot-and-fork",
            Self::NewWorkspace => "new-workspace",
        }
    }
}

#[derive(Debug, Clone)]
struct AgentOptions {
    direct: bool,
    socket_path: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Load {
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Save {
        #[arg(long)]
        path: Option<PathBuf>,
        #[arg(long)]
        overwrite: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ManifestCommand {
    Check {
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ProjectCommand {
    Add {
        path: PathBuf,
        #[arg(long)]
        manifest: Option<PathBuf>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Show {
        id_or_name: String,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Remove {
        id_or_name: String,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ProjectsCommand {
    List {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum WorkspaceCommand {
    Remove {
        id_or_path: String,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum SnapshotCommand {
    List {
        #[arg(long)]
        project: String,
        #[arg(long)]
        json: bool,
    },
    Show {
        snapshot_id: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        json: bool,
    },
    Export {
        snapshot_id: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum RecoverCommand {
    List {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Show {
        snapshot_id: String,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    Open {
        snapshot_id: String,
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        register: bool,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json_errors = cli.json_errors;
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            render_error(&err, json_errors);
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    let Cli {
        json_errors: _,
        direct,
        agent_socket,
        command,
    } = cli;
    let agent_options = AgentOptions {
        direct,
        socket_path: agent_socket,
    };

    match command {
        Command::Agent { command } => handle_agent_command(command)?,
        Command::Audit { command } => handle_audit_command(command)?,
        Command::Anchor { command } => handle_anchor_command(command)?,
        Command::Continue {
            source,
            target,
            manifest,
            config,
            dirty_policy,
            dry_run,
            json,
        } => {
            let (config_path, mut local_config) = load_or_default_config(config)?;
            let source_root = resolve_git_root(&source)?;
            let target_root = resolve_git_root(&target)?;
            let manifest_path = manifest.unwrap_or_else(|| source_root.join("devrelay.toml"));
            let manifest = Manifest::load(&manifest_path)
                .with_context(|| format!("failed to load {}", manifest_path.display()))?;
            let source_repo = GitRepo::new(&source_root);
            let target_repo = GitRepo::new(&target_root);
            let target_status = target_repo.status()?;

            if dry_run {
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "dry_run": true,
                            "source": source_root,
                            "target": target_root,
                            "project_id": manifest.project_id,
                            "target_status": target_status.summary(),
                            "dirty_policy": dirty_policy.label(),
                        }))?
                    );
                } else {
                    println!("continue dry-run: {}", manifest.project_id);
                    println!("  source: {}", source_root.display());
                    println!("  target: {}", target_root.display());
                    println!("  target: {}", target_status.short_summary());
                }
                return Ok(());
            }

            let home = DevRelayHome::resolve()?;
            home.create_base_dirs()?;
            let mut store = SnapshotStore::open(&home, &manifest.project_id)?;
            let source_snapshot = store.checkpoint(
                &source_repo,
                &manifest,
                false,
                Some("continue handoff source".to_string()),
            )?;
            let prepared =
                prepare_apply_target(&target_repo, &source_snapshot.metadata, dirty_policy)?;
            let snapshot_source = GitRepo::new(store.snapshot_repo_path());
            let verification =
                apply_snapshot(&prepared.repo, &snapshot_source, &source_snapshot.metadata)?;
            record_snapshot_apply_audit(
                &source_snapshot.metadata,
                prepared.repo.path(),
                "continue",
                dirty_policy,
                &prepared.backup,
                &verification,
            )?;
            let changed_states = mark_handoff_workspace_states(
                &mut local_config,
                &source_root,
                prepared.repo.path(),
            );
            if changed_states {
                local_config.save(&config_path)?;
            }

            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "continued": source_snapshot,
                        "source": source_root,
                        "target": prepared.repo.path(),
                        "dirty_policy": dirty_policy.label(),
                        "backup": prepared.backup,
                        "safe_actions": prepared.safe_actions,
                        "workspace_states_updated": changed_states,
                        "verification": verification,
                    }))?
                );
            } else {
                println!("continued: {}", source_snapshot.snapshot_id);
                println!("  source: {}", source_root.display());
                println!("  target: {}", prepared.repo.path().display());
                for action in prepared.safe_actions {
                    println!("  {action}");
                }
            }
        }
        Command::Config { command } => match command {
            ConfigCommand::Load { path, json } => {
                let path = config_path(path, false)?;
                let config = LocalConfig::load(&path)
                    .with_context(|| format!("failed to load {}", path.display()))?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&config)?);
                } else {
                    println!("config: {}", path.display());
                    println!("  fabric: {}", config.fabric_name);
                    println!("  device: {}", config.device_name);
                    println!("  projects: {}", config.project_registry.projects.len());
                }
            }
            ConfigCommand::Save {
                path,
                overwrite,
                json,
            } => {
                let path = config_path(path, true)?;
                if path.exists() && !overwrite {
                    return Err(DevRelayError::Config(format!(
                        "{} already exists; pass --overwrite to replace it",
                        path.display()
                    ))
                    .into());
                }
                let config = LocalConfig::new_for_local_device();
                config
                    .save(&path)
                    .with_context(|| format!("failed to save {}", path.display()))?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "saved": path,
                            "config": config,
                        }))?
                    );
                } else {
                    println!("saved config: {}", path.display());
                }
            }
        },
        Command::Diagnostics { command } => handle_diagnostics_command(command, &agent_options)?,
        Command::Doctor { command } => handle_doctor_command(command)?,
        Command::Device { command } => handle_device_command(command)?,
        Command::Devices { command } => handle_devices_command(command)?,
        Command::Discovery { command } => handle_discovery_command(command)?,
        Command::Environment { command } => handle_environment_command(command, &agent_options)?,
        Command::Metrics { command } => handle_metrics_command(command, &agent_options)?,
        Command::Identity { command } => handle_identity_command(command)?,
        Command::Manifest { command } => match command {
            ManifestCommand::Check { path, json } => {
                let manifest = Manifest::load(&path)
                    .with_context(|| format!("failed to load {}", path.display()))?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "ok": true,
                            "name": manifest.name,
                            "project_id": manifest.project_id,
                            "schema": manifest.schema,
                        }))?
                    );
                } else {
                    println!(
                        "ok: {} ({}) schema {}",
                        manifest.name, manifest.project_id, manifest.schema
                    );
                }
            }
        },
        Command::Pairing { command } => handle_pairing_command(command)?,
        Command::Project { command } => handle_project_command(command, &agent_options)?,
        Command::Projects { command } => handle_projects_command(command, &agent_options)?,
        Command::Workspace { command } => match command {
            WorkspaceCommand::Remove {
                id_or_path,
                config,
                json,
            } => {
                let (config_path, mut local_config) = load_or_default_config(config)?;
                let (project_id, removed) = remove_workspace(&mut local_config, &id_or_path)?;
                local_config.save(&config_path)?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "removed": removed,
                            "project_id": project_id,
                            "config": config_path,
                        }))?
                    );
                } else {
                    println!(
                        "removed workspace: {} ({})",
                        removed.workspace_id, project_id
                    );
                }
            }
        },
        Command::Recover { command } => handle_recover_command(command, &agent_options)?,
        Command::Session { command } => handle_session_command(command)?,
        Command::Sessions { command } => handle_sessions_command(command)?,
        Command::Snapshot { command } => match command {
            SnapshotCommand::List { project, json } => {
                let home = DevRelayHome::resolve()?;
                let store = SnapshotStore::open(&home, &project)?;
                let snapshots = store.list_snapshots()?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&snapshots)?);
                } else {
                    for snapshot in snapshots {
                        println!(
                            "{} #{}{}",
                            snapshot.snapshot_id,
                            snapshot.sequence_number,
                            if snapshot.pinned { " pinned" } else { "" }
                        );
                        if let Some(label) = snapshot.label {
                            println!("  label: {label}");
                        }
                    }
                }
            }
            SnapshotCommand::Show {
                snapshot_id,
                project,
                json,
            } => {
                let home = DevRelayHome::resolve()?;
                let store = SnapshotStore::open(&home, &project)?;
                let snapshot = store.get_snapshot(&snapshot_id)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&snapshot)?);
                } else {
                    println!(
                        "snapshot: {} #{}",
                        snapshot.snapshot_id, snapshot.sequence_number
                    );
                    println!("  project: {}", snapshot.project_id);
                    println!("  pinned: {}", snapshot.pinned);
                    if let Some(label) = snapshot.label {
                        println!("  label: {label}");
                    }
                    if let Some(parent) = snapshot.parent_snapshot_id {
                        println!("  parent: {parent}");
                    }
                }
            }
            SnapshotCommand::Export {
                snapshot_id,
                project,
                out,
                json,
            } => {
                let home = DevRelayHome::resolve()?;
                let store = SnapshotStore::open(&home, &project)?;
                let snapshot = store.export_snapshot_json(&snapshot_id, &out)?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "exported": snapshot,
                            "path": out,
                        }))?
                    );
                } else {
                    println!("exported snapshot: {}", snapshot.snapshot_id);
                    println!("  path: {}", out.display());
                }
            }
        },
        Command::Status {
            repo,
            manifest,
            json,
        } => handle_status(repo, manifest, json, &agent_options)?,
        Command::Checkpoint {
            repo,
            manifest,
            out,
            label,
            pin,
            json,
        } => handle_checkpoint(repo, manifest, out, label, pin, json, &agent_options)?,
        Command::Apply {
            repo,
            source,
            snapshot,
            dry_run,
            dirty_policy,
            json,
        } => handle_apply(
            repo,
            source,
            snapshot,
            dry_run,
            dirty_policy,
            json,
            &agent_options,
        )?,
    }
    Ok(())
}

fn handle_agent_command(command: AgentCommand) -> anyhow::Result<()> {
    match command {
        AgentCommand::Install {
            dry_run,
            agent_bin,
            service_dir,
            json,
        } => {
            let template = build_agent_service_template(agent_bin, service_dir)?;
            let commands = service_manual_commands(&template);
            if !dry_run {
                if let Some(parent) = template.service_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&template.service_path, &template.content)?;
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "dry_run": dry_run,
                        "installed": !dry_run,
                        "platform": template.kind.label(),
                        "service_path": template.service_path,
                        "content": template.content,
                        "manual_commands": commands,
                    }))?
                );
            } else {
                if dry_run {
                    println!("agent install dry-run: {}", template.kind.label());
                } else {
                    println!(
                        "agent service installed: {}",
                        template.service_path.display()
                    );
                }
                println!("  service: {}", template.service_path.display());
                for command in commands {
                    println!("  next: {command}");
                }
            }
        }
        AgentCommand::Uninstall { service_dir, json } => {
            let template = build_agent_service_template(None, service_dir)?;
            let existed = template.service_path.exists();
            if existed {
                fs::remove_file(&template.service_path)?;
            }
            let commands = service_uninstall_commands(&template);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "removed": existed,
                        "platform": template.kind.label(),
                        "service_path": template.service_path,
                        "manual_commands": commands,
                    }))?
                );
            } else {
                println!(
                    "agent service {}: {}",
                    if existed { "removed" } else { "not installed" },
                    template.service_path.display()
                );
                for command in commands {
                    println!("  next: {command}");
                }
            }
        }
        AgentCommand::Status { service_dir, json } => {
            let template = build_agent_service_template(None, service_dir)?;
            let installed = template.service_path.exists();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "installed": installed,
                        "platform": template.kind.label(),
                        "service_path": template.service_path,
                    }))?
                );
            } else {
                println!(
                    "agent service: {}",
                    if installed {
                        "installed"
                    } else {
                        "not installed"
                    }
                );
                println!("  platform: {}", template.kind.label());
                println!("  service: {}", template.service_path.display());
            }
        }
    }
    Ok(())
}

fn handle_audit_command(command: AuditCommand) -> anyhow::Result<()> {
    match command {
        AuditCommand::List {
            project,
            limit,
            include_sensitive_paths,
            json,
        } => {
            let (events, redactor) =
                collect_audit_events(project.as_deref(), limit, include_sensitive_paths)?;
            render_audit_list(&events, redactor.as_ref(), json)
        }
        AuditCommand::Export {
            out,
            project,
            limit,
            include_sensitive_paths,
            json,
        } => {
            let home = DevRelayHome::resolve()?;
            let path = out.unwrap_or_else(|| {
                home.diagnostics_dir()
                    .join(format!("audit-{}.json", devrelay_core::unix_now_seconds()))
            });
            let (events, redactor) =
                collect_audit_events(project.as_deref(), limit, include_sensitive_paths)?;
            let events_json = audit_events_json(&events, redactor.as_ref())?;
            let bundle = serde_json::json!({
                "schema_version": devrelay_core::AUDIT_SCHEMA_VERSION,
                "generated_at_unix_seconds": devrelay_core::unix_now_seconds(),
                "project": project,
                "include_sensitive_paths": include_sensitive_paths,
                "event_count": events.len(),
                "events": events_json,
            });
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, serde_json::to_vec_pretty(&bundle)?)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "path": path,
                        "event_count": events.len(),
                        "include_sensitive_paths": include_sensitive_paths,
                    }))?
                );
            } else {
                println!("audit exported: {}", path.display());
                println!("  events: {}", events.len());
                println!("  sensitive paths: {}", include_sensitive_paths);
            }
            Ok(())
        }
    }
}

fn collect_audit_events(
    project: Option<&str>,
    limit: usize,
    include_sensitive_paths: bool,
) -> anyhow::Result<(Vec<AuditEventRecord>, Option<LogRedactor>)> {
    let registry = open_device_registry()?;
    let home = DevRelayHome::resolve()?;
    let redactor = (!include_sensitive_paths)
        .then(|| LogRedactor::for_diagnostics(audit_local_paths(&home, &registry.config)));
    let project_ids = audit_project_ids(project, &registry.config)?;

    let mut events = registry.db.list_audit_events(project, limit)?;
    for project_id in project_ids {
        let db = MetadataDb::open(home.metadata_db_path(&project_id))?;
        events.extend(db.list_audit_events(Some(&project_id), limit)?);
    }
    events.sort_by(|left, right| {
        right
            .created_at_unix_seconds
            .cmp(&left.created_at_unix_seconds)
            .then(right.audit_id.cmp(&left.audit_id))
    });
    events.truncate(limit);
    Ok((events, redactor))
}

fn audit_project_ids(project: Option<&str>, config: &LocalConfig) -> anyhow::Result<Vec<String>> {
    if let Some(project) = project {
        return Ok(vec![find_project(config, project)?.project_id.clone()]);
    }
    Ok(config.project_registry.projects.keys().cloned().collect())
}

fn audit_local_paths(home: &DevRelayHome, config: &LocalConfig) -> Vec<PathBuf> {
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

fn render_audit_list(
    events: &[AuditEventRecord],
    redactor: Option<&LogRedactor>,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&audit_events_json(events, redactor)?)?
        );
    } else {
        for event in events {
            let summary = redactor
                .map(|redactor| redactor.redact_text(&event.summary))
                .unwrap_or_else(|| event.summary.clone());
            println!(
                "{} {} {} {}",
                event.created_at_unix_seconds,
                event.event_type.as_str(),
                event.outcome.as_str(),
                summary
            );
            if let Some(project_id) = &event.project_id {
                println!("  project: {project_id}");
            }
            if let Some(snapshot_id) = &event.snapshot_id {
                println!("  snapshot: {snapshot_id}");
            }
            if let Some(lease_id) = &event.lease_id {
                println!("  lease: {lease_id}");
            }
        }
    }
    Ok(())
}

fn audit_events_json(
    events: &[AuditEventRecord],
    redactor: Option<&LogRedactor>,
) -> anyhow::Result<serde_json::Value> {
    let events = events
        .iter()
        .map(|event| {
            let value = serde_json::to_value(event)?;
            Ok(match redactor {
                Some(redactor) => redactor.redact_json_value(value),
                None => value,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(serde_json::Value::Array(events))
}

#[derive(Debug, Serialize)]
struct AnchorStatusOutput {
    initialized: bool,
    role: AgentRole,
    anchor_mode: AnchorMode,
    home: PathBuf,
    config_path: PathBuf,
    layout: AnchorLayout,
    metadata_db_exists: bool,
    snapshot_repo_root_exists: bool,
    cas_root_exists: bool,
    startup_path_exists: bool,
}

#[derive(Debug, Serialize)]
struct AnchorMaintenanceOutput {
    project_id: String,
    gc_requested: bool,
    known_snapshot_count: usize,
    known_snapshot_ids: Vec<String>,
    report: AnchorSnapshotMaintenanceReport,
}

#[derive(Debug, Serialize)]
struct AnchorStartupRecord {
    version: u32,
    role: AgentRole,
    config_path: PathBuf,
    metadata_db_path: PathBuf,
    socket_path: PathBuf,
}

fn handle_anchor_command(command: AnchorCommand) -> anyhow::Result<()> {
    match command {
        AnchorCommand::Init { json } => {
            let status = init_anchor()?;
            render_anchor_status("anchor initialized", &status, json)
        }
        AnchorCommand::Status { json } => {
            let status = anchor_status()?;
            render_anchor_status("anchor status", &status, json)
        }
        AnchorCommand::Maintenance { project, gc, json } => {
            let output = anchor_maintenance(&project, gc)?;
            render_anchor_maintenance(&output, json)
        }
    }
}

fn init_anchor() -> anyhow::Result<AnchorStatusOutput> {
    let home = DevRelayHome::resolve()?;
    home.create_anchor_dirs()?;
    let config_path = home.config_file();
    let mut config = if config_path.exists() {
        LocalConfig::load(&config_path)?
    } else {
        LocalConfig::new_for_local_device()
    };
    config.anchor_mode = AnchorMode::UserSelected;
    config.save(&config_path)?;
    let _db = MetadataDb::open(home.anchor_metadata_db_path())?;
    write_anchor_startup_record(&home, &config_path)?;
    Ok(anchor_status_from_config(home, config_path, config))
}

fn anchor_status() -> anyhow::Result<AnchorStatusOutput> {
    let home = DevRelayHome::resolve()?;
    let config_path = home.config_file();
    let config = if config_path.exists() {
        LocalConfig::load(&config_path)?
    } else {
        LocalConfig::default()
    };
    Ok(anchor_status_from_config(home, config_path, config))
}

fn anchor_status_from_config(
    home: DevRelayHome,
    config_path: PathBuf,
    config: LocalConfig,
) -> AnchorStatusOutput {
    let layout = home.anchor_layout();
    let metadata_db_exists = layout.metadata_db_path.exists();
    let snapshot_repo_root_exists = layout.snapshot_repo_root.is_dir();
    let cas_root_exists = layout.cas_root.is_dir();
    let startup_path_exists = layout.startup_path.exists();
    let role = AgentRole::from_anchor_mode(config.anchor_mode);
    let initialized = role == AgentRole::Anchor
        && metadata_db_exists
        && snapshot_repo_root_exists
        && cas_root_exists
        && startup_path_exists;

    AnchorStatusOutput {
        initialized,
        role,
        anchor_mode: config.anchor_mode,
        home: home.root().to_path_buf(),
        config_path,
        layout,
        metadata_db_exists,
        snapshot_repo_root_exists,
        cas_root_exists,
        startup_path_exists,
    }
}

fn write_anchor_startup_record(home: &DevRelayHome, config_path: &Path) -> anyhow::Result<()> {
    let record = AnchorStartupRecord {
        version: 1,
        role: AgentRole::Anchor,
        config_path: config_path.to_path_buf(),
        metadata_db_path: home.anchor_metadata_db_path(),
        socket_path: home.agent_socket_path(),
    };
    fs::write(
        home.anchor_startup_path(),
        serde_json::to_vec_pretty(&record)?,
    )?;
    Ok(())
}

fn anchor_maintenance(project_id: &str, gc: bool) -> anyhow::Result<AnchorMaintenanceOutput> {
    let home = DevRelayHome::resolve()?;
    let anchor = AnchorSnapshotRepo::open_existing(&home, project_id)?;
    let known_snapshot_ids = anchor_known_snapshot_ids(&home, project_id)?;
    let report = if gc {
        anchor.run_guarded_gc(&known_snapshot_ids)?
    } else {
        anchor.inspect_maintenance(&known_snapshot_ids)?
    };
    Ok(AnchorMaintenanceOutput {
        project_id: project_id.to_string(),
        gc_requested: gc,
        known_snapshot_count: known_snapshot_ids.len(),
        known_snapshot_ids,
        report,
    })
}

fn anchor_known_snapshot_ids(home: &DevRelayHome, project_id: &str) -> anyhow::Result<Vec<String>> {
    let db = MetadataDb::open(home.anchor_metadata_db_path())?;
    let mut statement = db.connection().prepare(
        r#"
SELECT snapshot_id
FROM snapshots
WHERE project_id = ?1
ORDER BY sequence_number ASC, created_at_unix_seconds ASC
"#,
    )?;
    let rows = statement.query_map([project_id], |row| row.get::<_, String>(0))?;
    let mut snapshot_ids = Vec::new();
    for row in rows {
        snapshot_ids.push(row?);
    }
    Ok(snapshot_ids)
}

fn render_anchor_status(
    title: &str,
    status: &AnchorStatusOutput,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(status)?);
    } else {
        println!("{title}: {}", status.role.label());
        println!("  initialized: {}", status.initialized);
        println!("  home: {}", status.home.display());
        println!("  config: {}", status.config_path.display());
        println!("  metadata: {}", status.layout.metadata_db_path.display());
        println!(
            "  snapshots: {}",
            status.layout.snapshot_repo_root.display()
        );
        println!("  cas: {}", status.layout.cas_root.display());
        println!("  startup: {}", status.layout.startup_path.display());
    }
    Ok(())
}

fn render_anchor_maintenance(output: &AnchorMaintenanceOutput, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(output)?);
    } else {
        println!("anchor maintenance: {}", output.project_id);
        println!("  known snapshots: {}", output.known_snapshot_count);
        println!("  orphan refs: {}", output.report.orphan_refs.len());
        println!("  missing refs: {}", output.report.missing_refs.len());
        println!(
            "  repository bytes: {}",
            output.report.repository_size.total_bytes
        );
        println!("  gc ran: {}", output.report.gc_ran);
    }
    Ok(())
}

fn handle_diagnostics_command(
    command: DiagnosticsCommand,
    agent_options: &AgentOptions,
) -> anyhow::Result<()> {
    match command {
        DiagnosticsCommand::Export {
            out,
            include_sensitive_paths,
            json,
        } => {
            if agent_options.direct {
                return Err(DevRelayError::Ipc(
                    "diagnostics export is provided by the local DevRelay agent; remove --direct"
                        .to_string(),
                )
                .into());
            }
            let result: DiagnosticsExportResult = call_agent(
                agent_options,
                METHOD_DIAGNOSTICS_EXPORT,
                DiagnosticsExportParams {
                    out,
                    include_sensitive_paths,
                },
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                println!("diagnostics exported: {}", result.path.display());
                println!(
                    "  sensitive paths: {}",
                    if result.include_sensitive_paths {
                        "included"
                    } else {
                        "redacted"
                    }
                );
                println!("  source code included: {}", result.source_code_included);
                println!(
                    "  snapshot objects included: {}",
                    result.snapshot_objects_included
                );
            }
        }
    }
    Ok(())
}

fn handle_metrics_command(
    command: MetricsCommand,
    agent_options: &AgentOptions,
) -> anyhow::Result<()> {
    match command {
        MetricsCommand::Export {
            out,
            project,
            include_sensitive_paths,
            json,
        } => {
            let params = MetricsExportParams {
                out,
                project,
                include_sensitive_paths,
            };
            let result = if agent_options.direct {
                metrics_export_direct(params)?
            } else {
                call_agent(agent_options, METHOD_METRICS_EXPORT, params)?
            };
            render_metrics_export(&result, json)?;
        }
    }
    Ok(())
}

fn metrics_export_direct(params: MetricsExportParams) -> anyhow::Result<MetricsExportResult> {
    let (_, config) = load_or_default_config(None)?;
    let home = DevRelayHome::resolve()?;
    home.create_base_dirs()?;
    let path = params.out.clone().unwrap_or_else(|| {
        home.metrics_dir().join(format!(
            "metrics-{}.json",
            devrelay_core::unix_now_seconds()
        ))
    });
    let project_ids = audit_project_ids(params.project.as_deref(), &config)?;
    let mut report = collect_local_metrics_report(
        &home,
        params.project.clone(),
        &project_ids,
        devrelay_core::unix_now_seconds(),
        !params.include_sensitive_paths,
    )?;
    if !params.include_sensitive_paths {
        let redactor = LogRedactor::for_diagnostics(audit_local_paths(&home, &config));
        report = report.redact(&redactor);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
    Ok(MetricsExportResult {
        path,
        project: params.project,
        include_sensitive_paths: params.include_sensitive_paths,
        source_code_included: false,
        snapshot_objects_included: false,
        report,
    })
}

fn render_metrics_export(result: &MetricsExportResult, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(result)?);
    } else {
        println!("metrics exported: {}", result.path.display());
        if let Some(project) = &result.project {
            println!("  project: {project}");
        }
        println!(
            "  sensitive paths: {}",
            if result.include_sensitive_paths {
                "included"
            } else {
                "redacted"
            }
        );
        println!(
            "  continuation successes: {}",
            result.report.continuation.successes
        );
        println!("  checkpoints: {}", result.report.checkpoints.successes);
        println!("  source code included: {}", result.source_code_included);
        println!(
            "  snapshot objects included: {}",
            result.snapshot_objects_included
        );
    }
    Ok(())
}

fn handle_doctor_command(command: DoctorCommand) -> anyhow::Result<()> {
    match command {
        DoctorCommand::GitPerformance {
            repo,
            fix_safe,
            json,
        } => {
            let repo = GitRepo::new(resolve_git_root(&repo)?);
            let report = run_git_performance_doctor(&repo, fix_safe)?;
            render_git_performance_doctor(&report, json)
        }
        DoctorCommand::Paths {
            repo,
            manifest,
            target_platform,
            json,
        } => {
            let repo_root = resolve_git_root(&repo)?;
            let manifest_path = if manifest.is_absolute() {
                manifest
            } else {
                repo_root.join(manifest)
            };
            let manifest = Manifest::load(&manifest_path)
                .with_context(|| format!("failed to load {}", manifest_path.display()))?;
            let target_platform = target_platform.unwrap_or_else(current_platform_key);
            let report =
                run_path_portability_doctor(&GitRepo::new(repo_root), &manifest, &target_platform)?;
            render_path_portability_doctor(&report, json)
        }
        DoctorCommand::LineEndings {
            repo,
            target_platform,
            json,
        } => {
            let repo = GitRepo::new(resolve_git_root(&repo)?);
            let target_platform = target_platform.unwrap_or_else(current_platform_key);
            let report = run_line_ending_doctor(&repo, &target_platform)?;
            render_line_ending_doctor(&report, json)
        }
        DoctorCommand::Environment {
            repo,
            manifest,
            platform_key,
            secrets_config,
            config,
            run_healthcheck,
            allow_devcontainer_prepare,
            json,
        } => {
            let repo_root = resolve_git_root(&repo)?;
            let manifest_path = if manifest.is_absolute() {
                manifest
            } else {
                repo_root.join(manifest)
            };
            let manifest = Manifest::load(&manifest_path)
                .with_context(|| format!("failed to load {}", manifest_path.display()))?;
            let local_secrets = load_secret_provider_config(secrets_config.as_deref())?;
            let platform_key = platform_key.unwrap_or_else(current_platform_key);
            let mut options = EnvironmentDoctorOptions::for_platform(platform_key)
                .with_run_healthcheck(run_healthcheck)
                .with_allow_devcontainer_prepare(allow_devcontainer_prepare);
            if let Some(command_trust) =
                evaluate_environment_command_trust(&repo_root, &manifest, config)?
            {
                options = options.with_command_trust(command_trust);
            }
            let runner = SystemEnvironmentCommandRunner;
            let report =
                run_environment_doctor(&repo_root, &manifest, &local_secrets, &options, &runner)?;
            render_environment_doctor(&report, json)
        }
        DoctorCommand::WslFilesystem {
            repo,
            platform_key,
            json,
        } => {
            let repo = GitRepo::new(resolve_git_root(&repo)?);
            let platform_key = platform_key.unwrap_or_else(current_platform_key);
            let report = run_wsl_filesystem_doctor(&repo, &platform_key)?;
            render_wsl_filesystem_doctor(&report, json)
        }
    }
}

fn load_secret_provider_config(path: Option<&Path>) -> anyhow::Result<SecretProviderLocalConfig> {
    let Some(path) = path else {
        return Ok(SecretProviderLocalConfig::default());
    };
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        Ok(serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?)
    } else {
        Ok(toml::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))?)
    }
}

fn evaluate_environment_command_trust(
    repo_root: &Path,
    manifest: &Manifest,
    config: Option<PathBuf>,
) -> anyhow::Result<Option<devrelay_core::CommandTrustEvaluation>> {
    let home = DevRelayHome::resolve()?;
    let db_path = home.metadata_db_path(&manifest.project_id);
    if !db_path.exists() {
        return Ok(None);
    }
    let (_, local_config) = load_or_default_config(config)?;
    let command_hash = manifest.execution_trust_hash_with_files(repo_root)?;
    let db = MetadataDb::open(db_path)?;
    Ok(Some(db.evaluate_command_trust(
        &manifest.project_id,
        &local_config.device_id,
        "manifest",
        &command_hash,
    )?))
}

fn render_git_performance_doctor(
    report: &GitPerformanceDoctorReport,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
    } else {
        println!("git performance doctor: {}", report.repo.display());
        println!("  git: {}", report.git_version);
        println!(
            "  fsmonitor: supported={} config={}",
            report.fsmonitor_supported,
            report.fsmonitor_config.as_deref().unwrap_or("<unset>")
        );
        println!(
            "  untracked cache: supported={} config={}",
            report.untracked_cache_supported,
            report
                .untracked_cache_config
                .as_deref()
                .unwrap_or("<unset>")
        );
        for fix in &report.applied_fixes {
            println!("  applied: {}={}", fix.key, fix.value);
        }
        for fix in &report.skipped_fixes {
            println!("  skipped: {} already configured", fix.key);
        }
        for recommendation in &report.recommendations {
            println!("  recommendation: {}", recommendation.message);
        }
    }
    Ok(())
}

fn render_environment_doctor(report: &EnvironmentDoctorReport, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
    } else {
        println!("environment doctor: {}", report.repo.display());
        println!("  platform: {}", report.platform_key);
        println!(
            "  selected profile: {}",
            report.selected_profile_name.as_deref().unwrap_or("<none>")
        );
        println!(
            "  selected kind: {}",
            report
                .selected_profile_kind
                .map(|kind| format!("{kind:?}"))
                .unwrap_or_else(|| "<none>".to_string())
        );
        println!("  nix available: {}", report.nix_available);
        println!(
            "  container engine: {}",
            report.container_engine.as_deref().unwrap_or("<none>")
        );
        println!("  powershell available: {}", report.powershell_available);
        println!(
            "  required secrets: {}/{} mapped",
            report.mapped_required_secret_count, report.required_secret_count
        );
        if report.issues.is_empty() {
            println!("  issues: none");
        } else {
            println!("  issues: {}", report.issues.len());
            for issue in &report.issues {
                println!("  - {:?}: {}", issue.code, issue.message);
                if let Some(detail) = &issue.detail {
                    println!("    detail: {detail}");
                }
                for action in &issue.safe_actions {
                    println!("    action: {action}");
                }
            }
        }
        for line in &report.selection_explanation {
            println!("  selection: {line}");
        }
    }
    Ok(())
}

fn render_path_portability_doctor(
    report: &PathPortabilityDoctorReport,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
    } else {
        println!("path portability doctor: {}", report.repo.display());
        println!("  target: {}", report.target_platform_key);
        println!("  tracked paths: {}", report.tracked_count);
        println!(
            "  accepted untracked paths: {}",
            report.accepted_untracked_count
        );
        if report.issues.is_empty() {
            println!("  issues: none");
        } else {
            println!("  issues: {}", report.issues.len());
            for issue in &report.issues {
                println!("  - {:?}: {}", issue.code, issue.path);
                println!("    {}", issue.message);
                for action in &issue.safe_actions {
                    println!("    action: {action}");
                }
            }
        }
    }
    Ok(())
}

fn render_line_ending_doctor(report: &LineEndingDoctorReport, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
    } else {
        println!("line ending doctor: {}", report.repo.display());
        println!("  target: {}", report.target_platform_key);
        println!(
            "  .gitattributes: {}",
            if report.gitattributes_present {
                report.gitattributes_path.display().to_string()
            } else {
                "<missing>".to_string()
            }
        );
        println!(
            "  policy lines: {}",
            report.gitattributes_policy_lines.len()
        );
        println!(
            "  core.autocrlf: {}",
            report.core_autocrlf.as_deref().unwrap_or("<unset>")
        );
        println!("  tracked files checked: {}", report.tracked_file_count);
        println!(
            "  semantic hash mismatches: {}",
            report.semantic_hash_mismatches.len()
        );
        if report.warnings.is_empty() {
            println!("  warnings: none");
        } else {
            println!("  warnings: {}", report.warnings.len());
            for warning in &report.warnings {
                println!("  - {:?}: {}", warning.code, warning.message);
                for action in &warning.safe_actions {
                    println!("    action: {action}");
                }
            }
        }
    }
    Ok(())
}

fn render_wsl_filesystem_doctor(
    report: &WslFilesystemDoctorReport,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
    } else {
        println!("wsl filesystem doctor: {}", report.repo.display());
        println!("  platform: {}", report.platform_key);
        println!("  path kind: {:?}", report.path_kind);
        if report.warnings.is_empty() {
            println!("  warnings: none");
        } else {
            println!("  warnings: {}", report.warnings.len());
            for warning in &report.warnings {
                println!("  - {:?}: {}", warning.code, warning.message);
                for action in &warning.safe_actions {
                    println!("    action: {action}");
                }
            }
        }
        for item in &report.guidance {
            println!("  guidance: {item}");
        }
    }
    Ok(())
}

fn handle_devices_command(command: DevicesCommand) -> anyhow::Result<()> {
    match command {
        DevicesCommand::List { json } => {
            let registry = open_device_registry()?;
            let devices = registry.db.list_devices()?;
            render_devices_list(&devices, json)
        }
    }
}

fn handle_discovery_command(command: DiscoveryCommand) -> anyhow::Result<()> {
    match command {
        DiscoveryCommand::Advertise {
            role,
            port,
            dry_run,
            json,
        } => {
            let role = DiscoveryRole::from(role);
            let (bundle, registry) = open_identity_bundle(true)?;
            let advertisement = build_discovery_advertisement(
                role,
                &bundle.root.fabric_id,
                &bundle.device.device_id,
                port,
            )?;
            let mut advertised = false;
            if registry.config.mdns_enabled && !dry_run {
                let discovery = DiscoveryService::new()?;
                discovery.advertise(&advertisement)?;
                advertised = true;
            }
            render_discovery_advertisement(
                &advertisement,
                registry.config.mdns_enabled,
                dry_run,
                advertised,
                json,
            )
        }
        DiscoveryCommand::Browse {
            role,
            manual_address,
            json,
        } => {
            let role = DiscoveryRole::from(role);
            let registry = open_device_registry()?;
            let selected_manual_address =
                manual_address.or_else(|| registry.config.manual_discovery_address.clone());
            let mut browser_started = false;
            if selected_manual_address.is_none() {
                if !registry.config.mdns_enabled {
                    return Err(DevRelayError::Config(
                        "mDNS discovery is disabled and no manual address is configured"
                            .to_string(),
                    )
                    .into());
                }
                let discovery = DiscoveryService::new()?;
                let _receiver = discovery.browse(role)?;
                browser_started = true;
            }
            render_discovery_browser(
                role,
                registry.config.mdns_enabled,
                selected_manual_address,
                browser_started,
                json,
            )
        }
    }
}

fn render_discovery_advertisement(
    advertisement: &DiscoveryAdvertisement,
    mdns_enabled: bool,
    dry_run: bool,
    advertised: bool,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "dry_run": dry_run,
                "mdns_enabled": mdns_enabled,
                "advertised": advertised,
                "advertisement": advertisement,
            }))?
        );
    } else {
        println!("discovery advertisement: {}", advertisement.service_type);
        println!("  role: {}", advertisement.role.label());
        println!("  port: {}", advertisement.port);
        println!("  mDNS enabled: {mdns_enabled}");
        println!("  advertised: {advertised}");
        if dry_run {
            println!("  dry run: true");
        }
    }
    Ok(())
}

fn render_discovery_browser(
    role: DiscoveryRole,
    mdns_enabled: bool,
    manual_address: Option<String>,
    browser_started: bool,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "role": role,
                "service_type": role.service_type(),
                "mdns_enabled": mdns_enabled,
                "manual_address": manual_address,
                "browser_started": browser_started,
            }))?
        );
    } else {
        println!("discovery browser: {}", role.service_type());
        println!("  role: {}", role.label());
        println!("  mDNS enabled: {mdns_enabled}");
        if let Some(address) = manual_address {
            println!("  manual address: {address}");
        } else {
            println!("  browser started: {browser_started}");
        }
    }
    Ok(())
}

fn handle_environment_command(
    command: EnvironmentCommand,
    agent_options: &AgentOptions,
) -> anyhow::Result<()> {
    match command {
        EnvironmentCommand::Status {
            project,
            workspace,
            json,
        } => {
            let params = EnvironmentStatusParams { project, workspace };
            let result = if agent_options.direct {
                environment_status_direct(params)?
            } else {
                call_agent(agent_options, METHOD_ENVIRONMENT_STATUS, params)?
            };
            render_environment_status(&result, json)
        }
    }
}

fn environment_status_direct(
    params: EnvironmentStatusParams,
) -> anyhow::Result<EnvironmentStatusResult> {
    let (_, config) = load_or_default_config(None)?;
    let home = DevRelayHome::resolve()?;
    let mut environments = Vec::new();
    let projects = if let Some(project) = params.project.as_deref() {
        vec![find_project(&config, project)?.clone()]
    } else {
        config
            .project_registry
            .projects
            .values()
            .cloned()
            .collect::<Vec<_>>()
    };

    for project in projects {
        let workspace_ids =
            environment_workspace_ids_for_project(&project, params.workspace.as_deref());
        if workspace_ids.is_empty() && params.workspace.is_some() && params.project.is_some() {
            return Err(DevRelayError::Config(format!(
                "unknown workspace {} for project {}",
                params.workspace.as_deref().unwrap_or_default(),
                project.project_id
            ))
            .into());
        }
        for workspace_id in workspace_ids {
            let path = home.hydration_state_path(&project.project_id, workspace_id.as_deref());
            if path.exists() {
                environments.push(EnvironmentStatusEntry::from_persisted(
                    load_hydration_state(&path)?,
                ));
            } else {
                environments.push(EnvironmentStatusEntry::not_started(
                    project.project_id.clone(),
                    workspace_id,
                ));
            }
        }
    }

    if environments.is_empty()
        && let Some(workspace) = params.workspace
    {
        return Err(DevRelayError::Config(format!("unknown workspace {workspace}")).into());
    }

    environments.sort_by(|left, right| {
        left.record
            .project_id
            .cmp(&right.record.project_id)
            .then(left.record.workspace_id.cmp(&right.record.workspace_id))
    });
    Ok(EnvironmentStatusResult { environments })
}

fn environment_workspace_ids_for_project(
    project: &ProjectRegistryEntry,
    workspace: Option<&str>,
) -> Vec<Option<String>> {
    if let Some(workspace) = workspace {
        return project
            .workspaces
            .contains_key(workspace)
            .then(|| Some(workspace.to_string()))
            .into_iter()
            .collect();
    }
    if project.workspaces.is_empty() {
        return vec![None];
    }
    project.workspaces.keys().cloned().map(Some).collect()
}

fn render_environment_status(result: &EnvironmentStatusResult, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(result)?);
        return Ok(());
    }

    if result.environments.is_empty() {
        println!("environment status: no registered projects");
        return Ok(());
    }

    println!("environment status");
    for entry in &result.environments {
        let workspace = entry.record.workspace_id.as_deref().unwrap_or("project");
        let source = if entry.persisted {
            "persisted"
        } else {
            "not-started"
        };
        println!("  {} / {}", entry.record.project_id, workspace);
        println!("    state: {}", hydration_state_label(entry.record.state));
        println!("    attempt: {}", entry.record.attempt);
        println!("    source: {source}");
        if let Some(failure) = &entry.record.failure {
            println!("    failure: {failure}");
        }
        if entry.record.updated_at_unix_seconds > 0 {
            println!("    updated: {}", entry.record.updated_at_unix_seconds);
        }
    }
    Ok(())
}

fn hydration_state_label(state: devrelay_core::HydrationState) -> &'static str {
    match state {
        devrelay_core::HydrationState::Cold => "cold",
        devrelay_core::HydrationState::MetadataReady => "metadata-ready",
        devrelay_core::HydrationState::CacheReady => "cache-ready",
        devrelay_core::HydrationState::ShellReady => "shell-ready",
        devrelay_core::HydrationState::AppReady => "app-ready",
        devrelay_core::HydrationState::Failed => "failed",
    }
}

fn handle_identity_command(command: IdentityCommand) -> anyhow::Result<()> {
    match command {
        IdentityCommand::Init { json } => {
            let (bundle, _registry) = open_identity_bundle(true)?;
            render_identity_bundle(&bundle, json)
        }
        IdentityCommand::Show { json } => {
            let (bundle, _registry) = open_identity_bundle(false)?;
            render_identity_bundle(&bundle, json)
        }
        IdentityCommand::RecoveryExport { json } => {
            let status = devrelay_core::RecoveryExportStatus {
                available: false,
                message: "recovery export is reserved for M4 key backup".to_string(),
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!("recovery export: unavailable");
                println!("  {}", status.message);
            }
            Ok(())
        }
    }
}

fn open_identity_bundle(create: bool) -> anyhow::Result<(FabricIdentityBundle, DeviceRegistry)> {
    let registry = open_device_registry()?;
    let home = DevRelayHome::resolve()?;
    let store = FabricIdentityStore::new(home);
    let bundle = if create {
        store.open_or_create(&registry.config)?
    } else {
        store.public_bundle_from_store(&registry.config)?
    };
    registry.db.upsert_fabric_root_identity(&bundle.root)?;
    registry.db.upsert_device_public_identity(&bundle.device)?;
    Ok((bundle, registry))
}

fn render_identity_bundle(bundle: &FabricIdentityBundle, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(bundle)?);
    } else {
        println!(
            "fabric: {} ({})",
            bundle.root.fabric_name, bundle.root.fabric_id
        );
        println!("  root public key: {}", bundle.root.root_public_key_hex);
        println!(
            "device: {} ({})",
            bundle.device.display_name, bundle.device.device_id
        );
        println!(
            "  signing public key: {}",
            bundle.device.signing_public_key_hex
        );
        println!(
            "  network public key: {}",
            bundle.device.network_public_key_hex
        );
        println!(
            "  recovery export: {}",
            if bundle.recovery_export.available {
                "available"
            } else {
                "unavailable"
            }
        );
    }
    Ok(())
}

fn handle_pairing_command(command: PairingCommand) -> anyhow::Result<()> {
    match command {
        PairingCommand::Start {
            peer_device_id,
            peer_name,
            peer_signing_public_key,
            peer_network_public_key,
            peer_ephemeral_public_key,
            anchor,
            ttl_seconds,
            json,
        } => {
            let (bundle, mut registry) = open_identity_bundle(true)?;
            let session = registry.db.start_pairing_session(PairingStartRequest {
                fabric_id: &bundle.root.fabric_id,
                local_device_id: &bundle.device.device_id,
                peer_device_id: &peer_device_id,
                peer_display_name: &peer_name,
                peer_signing_public_key_hex: &peer_signing_public_key,
                peer_network_public_key_hex: &peer_network_public_key,
                peer_ephemeral_public_key_hex: &peer_ephemeral_public_key,
                anchor_address: anchor.as_deref(),
                ttl_seconds,
            })?;
            render_pairing_session(&session, json)
        }
        PairingCommand::Confirm {
            pairing_id,
            code,
            certificate_ttl_seconds,
            json,
        } => {
            let (bundle, registry) = open_identity_bundle(true)?;
            let session = registry
                .db
                .get_pairing_session(&pairing_id)?
                .ok_or_else(|| {
                    DevRelayError::Config(format!("unknown pairing session {pairing_id}"))
                })?;
            let peer = DevicePublicIdentity {
                device_id: session.peer_device_id.clone(),
                display_name: session.peer_display_name.clone(),
                fabric_id: session.fabric_id.clone(),
                signing_public_key_hex: session.peer_signing_public_key_hex.clone(),
                network_public_key_hex: session.peer_network_public_key_hex.clone(),
                platform_key: "unknown".to_string(),
                architecture: "unknown".to_string(),
                created_at_unix_seconds: session.created_at_unix_seconds,
                last_seen_unix_seconds: devrelay_core::unix_now_seconds(),
            };
            if peer.fabric_id != bundle.root.fabric_id {
                return Err(DevRelayError::Config(
                    "pairing session fabric does not match local identity".to_string(),
                )
                .into());
            }
            let home = DevRelayHome::resolve()?;
            let store = FabricIdentityStore::new(home);
            let now = devrelay_core::unix_now_seconds();
            let certificate =
                store.issue_device_certificate(&peer, now, certificate_ttl_seconds)?;
            let certificate_json = serde_json::to_string(&certificate)?;
            let mut db = registry.db;
            let confirmed =
                db.confirm_pairing_session(&pairing_id, &code, &certificate_json, now)?;
            render_pairing_session(&confirmed, json)
        }
        PairingCommand::Abort { pairing_id, json } => {
            let registry = open_device_registry()?;
            let mut db = registry.db;
            let aborted =
                db.abort_pairing_session(&pairing_id, devrelay_core::unix_now_seconds())?;
            render_pairing_session(&aborted, json)
        }
    }
}

fn render_pairing_session(session: &PairingSession, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(session)?);
    } else {
        println!(
            "pairing: {} ({})",
            session.pairing_id,
            session.state.as_str()
        );
        println!(
            "  peer: {} ({})",
            session.peer_display_name, session.peer_device_id
        );
        if let Some(anchor) = &session.anchor_address {
            println!("  anchor: {anchor}");
        }
        println!("  code: {}", session.short_authentication_string);
        println!("  expires: {}", session.expires_at_unix_seconds);
    }
    Ok(())
}

fn handle_device_command(command: DeviceCommand) -> anyhow::Result<()> {
    match command {
        DeviceCommand::Show { id, json } => {
            let registry = open_device_registry()?;
            let device_id = id.unwrap_or_else(|| registry.config.device_id.clone());
            let device = registry
                .db
                .get_device(&device_id)?
                .ok_or_else(|| DevRelayError::Config(format!("unknown device {device_id}")))?;
            render_device(&device, json)
        }
        DeviceCommand::Revoke {
            device_id,
            reason,
            key_rotation_required,
            json,
        } => {
            let mut registry = open_device_registry()?;
            let revocation = registry.db.revoke_device(
                &device_id,
                &registry.config.device_id,
                &reason,
                key_rotation_required,
            )?;
            render_device_revocation(&revocation, json)
        }
    }
}

struct DeviceRegistry {
    config: LocalConfig,
    db: MetadataDb,
}

fn open_device_registry() -> anyhow::Result<DeviceRegistry> {
    let (config_path, mut config) = load_or_default_config(None)?;
    config.mark_device_seen_now();
    config.save(&config_path)?;

    let home = DevRelayHome::resolve()?;
    if AgentRole::from_anchor_mode(config.anchor_mode) == AgentRole::Anchor {
        home.create_anchor_dirs()?;
    } else {
        home.create_base_dirs()?;
    }
    let db = MetadataDb::open(device_metadata_db_path(&home, &config))?;
    db.upsert_device_identity(&config.device_identity())?;
    Ok(DeviceRegistry { config, db })
}

fn device_metadata_db_path(home: &DevRelayHome, config: &LocalConfig) -> PathBuf {
    match AgentRole::from_anchor_mode(config.anchor_mode) {
        AgentRole::LocalOnly => home.root().join("agent.sqlite"),
        AgentRole::Anchor => home.anchor_metadata_db_path(),
    }
}

fn render_devices_list(devices: &[DeviceIdentity], json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(devices)?);
    } else {
        for device in devices {
            println!("{} ({})", device.display_name, device.device_id);
            println!("  platform: {}", device.platform_key);
            println!("  architecture: {}", device.architecture);
            println!("  last seen: {}", device.last_seen_unix_seconds);
        }
    }
    Ok(())
}

fn render_device(device: &DeviceIdentity, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(device)?);
    } else {
        println!("device: {} ({})", device.display_name, device.device_id);
        println!("  platform: {}", device.platform_key);
        println!("  architecture: {}", device.architecture);
        println!("  paired: {}", device.paired_at_unix_seconds.is_some());
        println!("  last seen: {}", device.last_seen_unix_seconds);
    }
    Ok(())
}

fn render_device_revocation(revocation: &DeviceRevocationRecord, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(revocation)?);
    } else {
        println!("revoked device: {}", revocation.device_id);
        println!("  by: {}", revocation.revoked_by_device_id);
        println!("  reason: {}", revocation.reason);
        println!(
            "  key rotation required: {}",
            revocation.key_rotation_required
        );
        println!("  revoked at: {}", revocation.revoked_at_unix_seconds);
    }
    Ok(())
}

fn handle_sessions_command(command: SessionsCommand) -> anyhow::Result<()> {
    match command {
        SessionsCommand::List { project, json } => {
            let sessions = list_sessions(project)?;
            render_sessions_list(&sessions, json)
        }
    }
}

fn handle_session_command(command: SessionCommand) -> anyhow::Result<()> {
    match command {
        SessionCommand::Show {
            session_id,
            project,
            json,
        } => {
            let (_, session) = find_session(project, &session_id)?;
            render_session(&session, json)
        }
        SessionCommand::Fork {
            session_id,
            project,
            name,
            json,
        } => {
            let (db, _) = find_session(project, &session_id)?;
            let session = db.fork_session(&session_id, &name)?;
            render_session(&session, json)
        }
        SessionCommand::Archive {
            session_id,
            project,
            json,
        } => {
            let (db, _) = find_session(project, &session_id)?;
            let session = db.archive_session(&session_id)?;
            render_session(&session, json)
        }
    }
}

fn list_sessions(project: Option<String>) -> anyhow::Result<Vec<StoredSession>> {
    let (_, config) = load_or_default_config(None)?;
    let home = DevRelayHome::resolve()?;
    let project_ids = session_project_ids(project.as_deref(), &config)?;
    let mut sessions = Vec::new();
    for project_id in project_ids {
        let db = MetadataDb::open(home.metadata_db_path(&project_id))?;
        sessions.extend(db.list_sessions(Some(&project_id))?);
    }
    sessions.sort_by(|left, right| {
        left.project_id
            .cmp(&right.project_id)
            .then(
                left.created_at_unix_seconds
                    .cmp(&right.created_at_unix_seconds),
            )
            .then(left.session_id.cmp(&right.session_id))
    });
    Ok(sessions)
}

fn find_session(
    project: Option<String>,
    session_id: &str,
) -> anyhow::Result<(MetadataDb, StoredSession)> {
    let (_, config) = load_or_default_config(None)?;
    let home = DevRelayHome::resolve()?;
    for project_id in session_project_ids(project.as_deref(), &config)? {
        let db = MetadataDb::open(home.metadata_db_path(&project_id))?;
        if let Some(session) = db.get_session(session_id)? {
            return Ok((db, session));
        }
    }
    Err(DevRelayError::Config(format!("unknown session {session_id}")).into())
}

fn session_project_ids(project: Option<&str>, config: &LocalConfig) -> anyhow::Result<Vec<String>> {
    if let Some(project) = project {
        let entry = find_project(config, project)?;
        return Ok(vec![entry.project_id.clone()]);
    }
    Ok(config.project_registry.projects.keys().cloned().collect())
}

fn ensure_default_session_for_project(project: &ProjectRegistryEntry) -> anyhow::Result<()> {
    let home = DevRelayHome::resolve()?;
    home.create_base_dirs()?;
    let db = MetadataDb::open(home.metadata_db_path(&project.project_id))?;
    db.ensure_default_session(&project.project_id, &project.display_name, None)?;
    Ok(())
}

fn render_sessions_list(sessions: &[StoredSession], json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(sessions)?);
    } else {
        for session in sessions {
            println!("{} ({})", session.name, session.session_id);
            println!("  project: {}", session.project_id);
            println!("  state: {}", session.state.as_str());
        }
    }
    Ok(())
}

fn render_session(session: &StoredSession, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(session)?);
    } else {
        println!("session: {} ({})", session.name, session.session_id);
        println!("  project: {}", session.project_id);
        println!("  state: {}", session.state.as_str());
        if let Some(parent) = &session.parent_session_id {
            println!("  parent: {parent}");
        }
        if let Some(archived_at) = session.archived_at_unix_seconds {
            println!("  archived at: {archived_at}");
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentServicePlatform {
    Macos,
    Linux,
}

fn build_agent_service_template(
    agent_bin: Option<PathBuf>,
    service_dir: Option<PathBuf>,
) -> anyhow::Result<ServiceTemplate> {
    let platform = current_agent_service_platform()?;
    let home = DevRelayHome::resolve()?;
    let agent_bin = agent_bin.unwrap_or(default_agent_bin()?);
    let service_dir = match service_dir {
        Some(path) => path,
        None => default_service_dir(platform)?,
    };
    let input = ServiceTemplateInput {
        agent_bin,
        devrelay_home: home.root().to_path_buf(),
        socket_path: home.agent_socket_path(),
        log_level: "info".to_string(),
    };
    Ok(match platform {
        AgentServicePlatform::Macos => macos_launch_agent_template(&input, &service_dir),
        AgentServicePlatform::Linux => linux_systemd_user_template(&input, &service_dir),
    })
}

fn current_agent_service_platform() -> anyhow::Result<AgentServicePlatform> {
    if cfg!(target_os = "macos") {
        Ok(AgentServicePlatform::Macos)
    } else if cfg!(target_os = "linux") {
        Ok(AgentServicePlatform::Linux)
    } else {
        Err(DevRelayError::Config(
            "devrelay agent install is not packaged for Windows yet; see docs/windows-startup.md"
                .to_string(),
        )
        .into())
    }
}

fn default_agent_bin() -> anyhow::Result<PathBuf> {
    let current = std::env::current_exe()?;
    Ok(current.with_file_name(if cfg!(windows) {
        "devrelay-agent.exe"
    } else {
        "devrelay-agent"
    }))
}

fn default_service_dir(platform: AgentServicePlatform) -> anyhow::Result<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        DevRelayError::Config("HOME must be set for agent service install".into())
    })?;
    match platform {
        AgentServicePlatform::Macos => Ok(home.join("Library").join("LaunchAgents")),
        AgentServicePlatform::Linux => {
            let config_home = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"));
            Ok(config_home.join("systemd").join("user"))
        }
    }
}

fn service_manual_commands(template: &ServiceTemplate) -> Vec<String> {
    match template.kind {
        ServiceTemplateKind::MacosLaunchAgent => vec![
            format!(
                "launchctl bootstrap gui/$(id -u) {}",
                template.service_path.display()
            ),
            "launchctl enable gui/$(id -u)/com.devrelay.agent".to_string(),
        ],
        ServiceTemplateKind::LinuxSystemdUser => vec![
            "systemctl --user daemon-reload".to_string(),
            "systemctl --user enable --now devrelay-agent.service".to_string(),
        ],
    }
}

fn service_uninstall_commands(template: &ServiceTemplate) -> Vec<String> {
    match template.kind {
        ServiceTemplateKind::MacosLaunchAgent => vec![
            "launchctl bootout gui/$(id -u)/com.devrelay.agent".to_string(),
            "launchctl disable gui/$(id -u)/com.devrelay.agent".to_string(),
        ],
        ServiceTemplateKind::LinuxSystemdUser => vec![
            "systemctl --user disable --now devrelay-agent.service".to_string(),
            "systemctl --user daemon-reload".to_string(),
        ],
    }
}

fn config_path(path: Option<PathBuf>, create_home: bool) -> anyhow::Result<PathBuf> {
    if let Some(path) = path {
        return Ok(path);
    }
    let home = DevRelayHome::resolve()?;
    if create_home {
        home.create_base_dirs()?;
    }
    Ok(home.config_file())
}

fn load_or_default_config(path: Option<PathBuf>) -> anyhow::Result<(PathBuf, LocalConfig)> {
    let path = config_path(path, true)?;
    if path.exists() {
        Ok((path.clone(), LocalConfig::load(&path)?))
    } else {
        Ok((path, LocalConfig::new_for_local_device()))
    }
}

fn handle_project_command(
    command: ProjectCommand,
    agent_options: &AgentOptions,
) -> anyhow::Result<()> {
    match command {
        ProjectCommand::Add {
            path,
            manifest,
            config,
            json,
        } => {
            let (added, config_path) =
                if should_route_config_command(agent_options, config.as_ref()) {
                    (
                        project_add_via_agent(agent_options, path, manifest)?,
                        config_path(None, false)?,
                    )
                } else {
                    project_add_direct(path, manifest, config)?
                };
            render_project_add(&added, &config_path, json)
        }
        ProjectCommand::Show {
            id_or_name,
            config,
            json,
        } => {
            let entry = if should_route_config_command(agent_options, config.as_ref()) {
                project_show_via_agent(agent_options, id_or_name)?
            } else {
                project_show_direct(id_or_name, config)?
            };
            render_project_show(&entry, json)
        }
        ProjectCommand::Remove {
            id_or_name,
            config,
            json,
        } => {
            let (removed, config_path) =
                if should_route_config_command(agent_options, config.as_ref()) {
                    (
                        project_remove_via_agent(agent_options, id_or_name)?,
                        config_path(None, false)?,
                    )
                } else {
                    project_remove_direct(id_or_name, config)?
                };
            render_project_remove(&removed, &config_path, json)
        }
    }
}

fn handle_projects_command(
    command: ProjectsCommand,
    agent_options: &AgentOptions,
) -> anyhow::Result<()> {
    match command {
        ProjectsCommand::List { config, json } => {
            let projects = if should_route_config_command(agent_options, config.as_ref()) {
                projects_list_via_agent(agent_options)?
            } else {
                projects_list_direct(config)?
            };
            render_projects_list(&projects, json)
        }
    }
}

fn should_route_config_command(agent_options: &AgentOptions, config: Option<&PathBuf>) -> bool {
    !agent_options.direct && config.is_none()
}

fn project_add_direct(
    path: PathBuf,
    manifest: Option<PathBuf>,
    config: Option<PathBuf>,
) -> anyhow::Result<(ProjectRegistryEntry, PathBuf)> {
    let (config_path, mut local_config) = load_or_default_config(config)?;
    refresh_workspace_states(&mut local_config);
    let entry = build_project_registry_entry(&path, manifest.as_deref(), &local_config.device_id)?;
    ensure_workspace_not_registered(&local_config, &entry.local_path)?;
    let project_id = entry.project_id.clone();
    merge_project_registry_entry(&mut local_config, entry);
    let added = local_config
        .project_registry
        .projects
        .get(&project_id)
        .cloned()
        .ok_or_else(|| DevRelayError::Config("project disappeared".to_string()))?;
    let home = DevRelayHome::resolve()?;
    ensure_anchor_project_repo_for_config(&home, &local_config, &added.project_id)?;
    local_config.save(&config_path)?;
    ensure_default_session_for_project(&added)?;
    Ok((added, config_path))
}

fn ensure_anchor_project_repo_for_config(
    home: &DevRelayHome,
    config: &LocalConfig,
    project_id: &str,
) -> anyhow::Result<()> {
    if AgentRole::from_anchor_mode(config.anchor_mode) == AgentRole::Anchor {
        AnchorSnapshotRepo::open(home, project_id)?;
    }
    Ok(())
}

fn project_show_direct(
    id_or_name: String,
    config: Option<PathBuf>,
) -> anyhow::Result<ProjectRegistryEntry> {
    let (config_path, mut local_config) = load_or_default_config(config)?;
    if refresh_workspace_states(&mut local_config) {
        local_config.save(&config_path)?;
    }
    Ok(find_project(&local_config, &id_or_name)?.clone())
}

fn project_remove_direct(
    id_or_name: String,
    config: Option<PathBuf>,
) -> anyhow::Result<(ProjectRegistryEntry, PathBuf)> {
    let (config_path, mut local_config) = load_or_default_config(config)?;
    let project_id = find_project(&local_config, &id_or_name)?.project_id.clone();
    let removed = local_config
        .project_registry
        .projects
        .remove(&project_id)
        .ok_or_else(|| DevRelayError::Config("project disappeared".to_string()))?;
    local_config.save(&config_path)?;
    Ok((removed, config_path))
}

fn projects_list_direct(config: Option<PathBuf>) -> anyhow::Result<Vec<ProjectRegistryEntry>> {
    let (config_path, mut local_config) = load_or_default_config(config)?;
    if refresh_workspace_states(&mut local_config) {
        local_config.save(&config_path)?;
    }
    Ok(local_config
        .project_registry
        .projects
        .values()
        .cloned()
        .collect())
}

fn project_add_via_agent(
    agent_options: &AgentOptions,
    path: PathBuf,
    manifest: Option<PathBuf>,
) -> anyhow::Result<ProjectRegistryEntry> {
    let path = absolute_cli_path(path)?;
    let manifest = manifest.map(absolute_cli_path).transpose()?;
    let result: ProjectResult = call_agent(
        agent_options,
        METHOD_PROJECTS_ADD,
        ProjectsAddParams { path, manifest },
    )?;
    Ok(result.project)
}

fn project_show_via_agent(
    agent_options: &AgentOptions,
    id_or_name: String,
) -> anyhow::Result<ProjectRegistryEntry> {
    let result: ProjectResult = call_agent(
        agent_options,
        METHOD_PROJECTS_SHOW,
        ProjectsShowParams { id_or_name },
    )?;
    Ok(result.project)
}

fn project_remove_via_agent(
    agent_options: &AgentOptions,
    id_or_name: String,
) -> anyhow::Result<ProjectRegistryEntry> {
    let result: ProjectResult = call_agent(
        agent_options,
        METHOD_PROJECTS_REMOVE,
        ProjectsRemoveParams { id_or_name },
    )?;
    Ok(result.project)
}

fn projects_list_via_agent(
    agent_options: &AgentOptions,
) -> anyhow::Result<Vec<ProjectRegistryEntry>> {
    let result: ProjectsListResult =
        call_agent(agent_options, METHOD_PROJECTS_LIST, serde_json::json!({}))?;
    Ok(result.projects)
}

fn render_project_add(
    added: &ProjectRegistryEntry,
    config_path: &Path,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "added": added,
                "config": config_path,
            }))?
        );
    } else {
        println!(
            "added project: {} ({})",
            added.display_name, added.project_id
        );
        println!("  path: {}", added.local_path.display());
        println!("  workspaces: {}", added.workspaces.len());
    }
    Ok(())
}

fn render_project_show(entry: &ProjectRegistryEntry, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(entry)?);
    } else {
        println!("project: {} ({})", entry.display_name, entry.project_id);
        println!("  path: {}", entry.local_path.display());
        println!("  workspaces: {}", entry.workspaces.len());
        for workspace in entry.workspaces.values() {
            println!(
                "    {} [{}] {}",
                workspace.workspace_id,
                workspace_state_label(workspace.state),
                workspace.local_path.display()
            );
        }
    }
    Ok(())
}

fn render_project_remove(
    removed: &ProjectRegistryEntry,
    config_path: &Path,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "removed": removed,
                "config": config_path,
            }))?
        );
    } else {
        println!(
            "removed project: {} ({})",
            removed.display_name, removed.project_id
        );
    }
    Ok(())
}

fn render_projects_list(projects: &[ProjectRegistryEntry], json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(projects)?);
    } else {
        for project in projects {
            println!("{} ({})", project.display_name, project.project_id);
            println!("  path: {}", project.local_path.display());
            println!("  workspaces: {}", project.workspaces.len());
        }
    }
    Ok(())
}

fn handle_checkpoint(
    repo: PathBuf,
    manifest: PathBuf,
    out: Option<PathBuf>,
    label: Option<String>,
    pin: bool,
    json: bool,
    agent_options: &AgentOptions,
) -> anyhow::Result<()> {
    let result = if agent_options.direct {
        checkpoint_direct(repo, manifest, label, pin)?
    } else {
        checkpoint_via_agent(agent_options, repo, manifest, label, pin)?
    };
    if let Some(out) = &out {
        write_snapshot_file(out, &result.checkpoint.metadata)?;
    }
    render_checkpoint(&result, out.as_ref(), json)
}

fn checkpoint_direct(
    repo: PathBuf,
    manifest: PathBuf,
    label: Option<String>,
    pin: bool,
) -> anyhow::Result<CheckpointCreateResult> {
    let manifest = Manifest::load(&manifest)
        .with_context(|| format!("failed to load {}", manifest.display()))?;
    let repo = GitRepo::new(repo);
    let home = DevRelayHome::resolve()?;
    home.create_base_dirs()?;
    let mut store = SnapshotStore::open(&home, &manifest.project_id)?;
    let checkpoint = store.checkpoint(&repo, &manifest, pin, label)?;
    Ok(CheckpointCreateResult {
        checkpoint,
        snapshot_repo: store.snapshot_repo_path().to_path_buf(),
    })
}

fn checkpoint_via_agent(
    agent_options: &AgentOptions,
    repo: PathBuf,
    manifest: PathBuf,
    label: Option<String>,
    pin: bool,
) -> anyhow::Result<CheckpointCreateResult> {
    call_agent(
        agent_options,
        METHOD_CHECKPOINT_CREATE,
        CheckpointCreateParams {
            repo: absolute_cli_path(repo)?,
            manifest: Some(absolute_cli_path(manifest)?),
            label,
            pin,
        },
    )
}

fn render_checkpoint(
    result: &CheckpointCreateResult,
    out: Option<&PathBuf>,
    json: bool,
) -> anyhow::Result<()> {
    let stored = &result.checkpoint;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "checkpoint": stored,
                "snapshot_file": out,
                "snapshot_repo": result.snapshot_repo,
            }))?
        );
    } else {
        println!("checkpoint: {}", stored.snapshot_id);
        println!("  sequence: {}", stored.sequence_number);
        println!("  pinned: {}", stored.pinned);
        if let Some(label) = &stored.label {
            println!("  label: {label}");
        }
        println!("  head: {}", stored.metadata.head_oid);
        println!("  index: {}", stored.metadata.index_tree_oid);
        println!("  work: {}", stored.metadata.work_tree_oid);
        println!(
            "  included untracked: {}",
            stored.metadata.included_untracked.len()
        );
        println!("  snapshot repo: {}", result.snapshot_repo.display());
        if let Some(out) = out {
            println!("  snapshot file: {}", out.display());
        }
    }
    Ok(())
}

fn handle_recover_command(
    command: RecoverCommand,
    agent_options: &AgentOptions,
) -> anyhow::Result<()> {
    match command {
        RecoverCommand::List {
            project,
            config,
            json,
        } => {
            let snapshots = if should_route_config_command(agent_options, config.as_ref()) {
                recover_list_via_agent(agent_options, project)?
            } else {
                recover_list_direct(project, config)?
            };
            render_recover_list(&snapshots, json)
        }
        RecoverCommand::Show {
            snapshot_id,
            project,
            config,
            json,
        } => {
            let snapshot = if should_route_config_command(agent_options, config.as_ref()) {
                recover_show_via_agent(agent_options, snapshot_id, project)?
            } else {
                recover_show_direct(snapshot_id, project, config)?
            };
            render_recover_show(&snapshot, json)
        }
        RecoverCommand::Open {
            snapshot_id,
            path,
            project,
            config,
            register,
            name,
            json,
        } => {
            let result = if should_route_config_command(agent_options, config.as_ref()) {
                recover_open_via_agent(agent_options, snapshot_id, path, project, register, name)?
            } else {
                recover_open_direct(snapshot_id, path, project, config, register, name)?
            };
            render_recover_open(&result, json)
        }
    }
}

fn recover_list_direct(
    project: Option<String>,
    config: Option<PathBuf>,
) -> anyhow::Result<Vec<StoredSnapshot>> {
    let (_, local_config) = load_or_default_config(config)?;
    let home = DevRelayHome::resolve()?;
    recover_list_snapshots(&home, &local_config, project.as_deref())
}

fn recover_show_direct(
    snapshot_id: String,
    project: Option<String>,
    config: Option<PathBuf>,
) -> anyhow::Result<StoredSnapshot> {
    let (_, local_config) = load_or_default_config(config)?;
    let home = DevRelayHome::resolve()?;
    let (_, _, snapshot) =
        find_recovery_snapshot(&home, &local_config, project.as_deref(), &snapshot_id)?;
    Ok(snapshot)
}

fn recover_open_direct(
    snapshot_id: String,
    path: PathBuf,
    project: Option<String>,
    config: Option<PathBuf>,
    register: bool,
    name: Option<String>,
) -> anyhow::Result<RecoverOpenResult> {
    let (config_path, mut local_config) = load_or_default_config(config)?;
    let home = DevRelayHome::resolve()?;
    let (project_entry, store, snapshot) =
        find_recovery_snapshot(&home, &local_config, project.as_deref(), &snapshot_id)?;
    let source_path = recovery_source_path(&project_entry)?;
    let target = prepare_recovery_workspace(&path, &source_path)?;
    let snapshot_source = GitRepo::new(store.snapshot_repo_path());
    let verification = apply_snapshot(&target, &snapshot_source, &snapshot.metadata)?;
    record_snapshot_apply_audit(
        &snapshot.metadata,
        target.path(),
        "recover.open",
        DirtyPolicy::Block,
        &None,
        &verification,
    )?;
    let registered = if register {
        let workspace = register_recovery_workspace(
            &mut local_config,
            &project_entry.project_id,
            target.path(),
        )?;
        local_config.save(&config_path)?;
        Some(workspace)
    } else {
        None
    };
    Ok(RecoverOpenResult {
        recovered: snapshot,
        path: target.path().to_path_buf(),
        name,
        registered,
        verification,
    })
}

fn recover_list_via_agent(
    agent_options: &AgentOptions,
    project: Option<String>,
) -> anyhow::Result<Vec<StoredSnapshot>> {
    let result: RecoverListResult = call_agent(
        agent_options,
        METHOD_RECOVER_LIST,
        RecoverListParams { project },
    )?;
    Ok(result.snapshots)
}

fn recover_show_via_agent(
    agent_options: &AgentOptions,
    snapshot_id: String,
    project: Option<String>,
) -> anyhow::Result<StoredSnapshot> {
    let result: RecoverShowResult = call_agent(
        agent_options,
        METHOD_RECOVER_SHOW,
        RecoverShowParams {
            snapshot_id,
            project,
        },
    )?;
    Ok(result.snapshot)
}

fn recover_open_via_agent(
    agent_options: &AgentOptions,
    snapshot_id: String,
    path: PathBuf,
    project: Option<String>,
    register: bool,
    name: Option<String>,
) -> anyhow::Result<RecoverOpenResult> {
    call_agent(
        agent_options,
        METHOD_RECOVER_OPEN,
        RecoverOpenParams {
            snapshot_id,
            path: absolute_cli_path(path)?,
            project,
            register,
            name,
        },
    )
}

fn render_recover_list(snapshots: &[StoredSnapshot], json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(snapshots)?);
    } else {
        for snapshot in snapshots {
            println!(
                "{} {} #{}",
                snapshot.project_id, snapshot.snapshot_id, snapshot.sequence_number
            );
        }
    }
    Ok(())
}

fn render_recover_show(snapshot: &StoredSnapshot, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(snapshot)?);
    } else {
        println!(
            "snapshot: {} #{}",
            snapshot.snapshot_id, snapshot.sequence_number
        );
        println!("  project: {}", snapshot.project_id);
        if let Some(label) = &snapshot.label {
            println!("  label: {label}");
        }
    }
    Ok(())
}

fn render_recover_open(result: &RecoverOpenResult, json: bool) -> anyhow::Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(result)?);
    } else {
        println!("recovered snapshot: {}", result.recovered.snapshot_id);
        println!("  path: {}", result.path.display());
        if let Some(name) = &result.name {
            println!("  name: {name}");
        }
        if result.registered.is_some() {
            println!("  registered: true");
        }
    }
    Ok(())
}

fn handle_apply(
    repo: PathBuf,
    source: PathBuf,
    snapshot: PathBuf,
    dry_run: bool,
    dirty_policy: DirtyPolicy,
    json: bool,
    agent_options: &AgentOptions,
) -> anyhow::Result<()> {
    if should_route_apply_command(agent_options, dirty_policy) {
        let applied_repo = repo.clone();
        let result = apply_via_agent(agent_options, repo, snapshot, dry_run)?;
        render_agent_apply_result(&result, &applied_repo, dirty_policy, dry_run, json)
    } else {
        apply_direct(repo, source, snapshot, dry_run, dirty_policy, json)
    }
}

fn should_route_apply_command(agent_options: &AgentOptions, dirty_policy: DirtyPolicy) -> bool {
    !agent_options.direct && dirty_policy == DirtyPolicy::Block
}

fn apply_direct(
    repo: PathBuf,
    source: PathBuf,
    snapshot: PathBuf,
    dry_run: bool,
    dirty_policy: DirtyPolicy,
    json: bool,
) -> anyhow::Result<()> {
    let snapshot = read_snapshot_file(&snapshot)
        .with_context(|| format!("failed to read {}", snapshot.display()))?;
    let target = GitRepo::new(repo);
    let source = GitRepo::new(source);
    if dry_run {
        let plan = plan_apply_snapshot(&target, &source, &snapshot)?;
        render_apply_dry_run(&plan, json)
    } else {
        let prepared = prepare_apply_target(&target, &snapshot, dirty_policy)?;
        let verification = apply_snapshot(&prepared.repo, &source, &snapshot)?;
        record_snapshot_apply_audit(
            &snapshot,
            prepared.repo.path(),
            "apply",
            dirty_policy,
            &prepared.backup,
            &verification,
        )?;
        render_apply_success(
            &snapshot.snapshot_id,
            prepared.repo.path(),
            dirty_policy,
            &prepared.backup,
            &prepared.safe_actions,
            &verification,
            json,
        )
    }
}

fn record_snapshot_apply_audit(
    snapshot: &SnapshotMetadata,
    target_path: &Path,
    operation: &str,
    dirty_policy: DirtyPolicy,
    backup: &Option<StoredSnapshot>,
    verification: &devrelay_core::VerificationDetails,
) -> anyhow::Result<()> {
    let home = DevRelayHome::resolve()?;
    let db = MetadataDb::open(home.metadata_db_path(&snapshot.project_id))?;
    let target_path = target_path.to_string_lossy().to_string();
    let mut audit = AuditEventInput::new(
        AuditEventType::SnapshotApplied,
        AuditOutcome::Succeeded,
        "snapshot applied to workspace",
    )
    .with_detail(serde_json::json!({
        "operation": operation,
        "target_path": target_path,
        "dirty_policy": dirty_policy.label(),
        "backup_snapshot_id": backup.as_ref().map(|snapshot| snapshot.snapshot_id.as_str()),
        "verified_state_hash": verification.state_hash.as_str(),
        "included_untracked_count": verification.included_untracked.len(),
        "excluded_path_count": verification.excluded_paths.len(),
    }));
    audit.project_id = Some(snapshot.project_id.clone());
    audit.session_id = snapshot.session_id.clone();
    audit.snapshot_id = Some(snapshot.snapshot_id.clone());
    db.record_audit_event(audit)?;
    Ok(())
}

fn apply_via_agent(
    agent_options: &AgentOptions,
    repo: PathBuf,
    snapshot_path: PathBuf,
    dry_run: bool,
) -> anyhow::Result<ApplySnapshotResult> {
    let snapshot = read_snapshot_file(&snapshot_path)
        .with_context(|| format!("failed to read {}", snapshot_path.display()))?;
    call_agent(
        agent_options,
        METHOD_APPLY_SNAPSHOT,
        ApplySnapshotParams {
            repo: absolute_cli_path(repo)?,
            project: snapshot.project_id,
            snapshot_id: snapshot.snapshot_id,
            dry_run,
        },
    )
}

fn render_agent_apply_result(
    result: &ApplySnapshotResult,
    applied_repo: &Path,
    dirty_policy: DirtyPolicy,
    dry_run: bool,
    json: bool,
) -> anyhow::Result<()> {
    if dry_run {
        let plan = result
            .plan
            .as_ref()
            .ok_or_else(|| DevRelayError::Ipc("agent apply response missing plan".to_string()))?;
        render_apply_dry_run(plan, json)
    } else {
        let verification = result.verification.as_ref().ok_or_else(|| {
            DevRelayError::Ipc("agent apply response missing verification".to_string())
        })?;
        render_apply_success(
            &result.snapshot.snapshot_id,
            applied_repo,
            dirty_policy,
            &None,
            &[],
            verification,
            json,
        )
    }
}

fn render_apply_dry_run(plan: &devrelay_core::ApplyPlan, json: bool) -> anyhow::Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "dry_run": true,
                "plan": plan,
            }))?
        );
    } else {
        println!("apply dry-run: {}", plan.snapshot_id);
        println!("  head: {}", plan.head_oid);
        println!(
            "  target: {}",
            plan.branch.as_deref().unwrap_or("detached HEAD")
        );
    }
    Ok(())
}

fn render_apply_success(
    snapshot_id: &str,
    applied_repo: &Path,
    dirty_policy: DirtyPolicy,
    backup: &Option<StoredSnapshot>,
    safe_actions: &[String],
    verification: &devrelay_core::VerificationDetails,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "applied": snapshot_id,
                "applied_repo": applied_repo,
                "dirty_policy": dirty_policy.label(),
                "backup": backup,
                "safe_actions": safe_actions,
                "verification": verification,
            }))?
        );
    } else {
        println!("applied: {snapshot_id}");
        println!("  repo: {}", applied_repo.display());
        for action in safe_actions {
            println!("  {action}");
        }
    }
    Ok(())
}

fn build_project_registry_entry(
    path: &Path,
    manifest_path: Option<&Path>,
    device_id: &str,
) -> anyhow::Result<devrelay_core::ProjectRegistryEntry> {
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

    Ok(devrelay_core::ProjectRegistryEntry {
        project_id,
        display_name,
        local_path: root,
        workspaces: BTreeMap::from([(workspace_id, workspace)]),
        manifest_path,
        remote_url_fingerprint: remote_fingerprint(&repo),
        root_commit_fingerprint: root_commit_fingerprint(&repo),
    })
}

fn merge_project_registry_entry(
    config: &mut LocalConfig,
    entry: devrelay_core::ProjectRegistryEntry,
) {
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

fn resolve_git_root(path: &Path) -> anyhow::Result<PathBuf> {
    let repo = GitRepo::new(path);
    let raw = repo
        .run(&["rev-parse", "--show-toplevel"])
        .map_err(|_| DevRelayError::NotGitRepository(path.display().to_string()))?;
    Ok(PathBuf::from(raw))
}

fn handle_status(
    repo: PathBuf,
    manifest: PathBuf,
    json: bool,
    agent_options: &AgentOptions,
) -> anyhow::Result<()> {
    let repo = absolute_cli_path(repo)?;
    let manifest_path = absolute_cli_path(manifest)?;
    let manifest = Manifest::load(&manifest_path)
        .with_context(|| format!("failed to load {}", manifest_path.display()))?;
    let result = if agent_options.direct {
        collect_status_direct(&repo, &manifest)?
    } else {
        collect_status_via_agent(agent_options, repo, Some(manifest_path))?
    };
    render_status_result(&manifest.name, &result, json)
}

fn collect_status_direct(repo_path: &Path, manifest: &Manifest) -> anyhow::Result<StatusGetResult> {
    let repo = GitRepo::new(repo_path);
    let status = repo.status()?;
    let untracked = classify_untracked_paths(repo.path(), manifest, status.untracked_paths())?;
    Ok(StatusGetResult {
        status: status.summary(),
        entries: status.entries,
        untracked,
    })
}

#[cfg(unix)]
fn collect_status_via_agent(
    agent_options: &AgentOptions,
    repo: PathBuf,
    manifest: Option<PathBuf>,
) -> anyhow::Result<StatusGetResult> {
    call_agent(
        agent_options,
        METHOD_STATUS_GET,
        StatusGetParams { repo, manifest },
    )
}

#[cfg(not(unix))]
fn collect_status_via_agent(
    _agent_options: &AgentOptions,
    repo: PathBuf,
    manifest: Option<PathBuf>,
) -> anyhow::Result<StatusGetResult> {
    let manifest_path = manifest
        .ok_or_else(|| DevRelayError::Manifest("status requires a manifest path".to_string()))?;
    let manifest = Manifest::load(&manifest_path)
        .with_context(|| format!("failed to load {}", manifest_path.display()))?;
    collect_status_direct(&repo, &manifest)
}

#[cfg(unix)]
fn call_agent<P, R>(agent_options: &AgentOptions, method: &str, params: P) -> anyhow::Result<R>
where
    P: serde::Serialize,
    R: serde::de::DeserializeOwned,
{
    let socket = agent_options
        .socket_path
        .clone()
        .map(Ok)
        .unwrap_or_else(|| DevRelayHome::resolve().map(|home| home.agent_socket_path()))?;
    let client = AgentRpcClient::new(&socket);
    client
        .call(method, params)
        .map_err(|err| agent_rpc_error(&socket, err).into())
}

#[cfg(not(unix))]
fn call_agent<P, R>(_agent_options: &AgentOptions, _method: &str, _params: P) -> anyhow::Result<R>
where
    P: serde::Serialize,
    R: serde::de::DeserializeOwned,
{
    Err(DevRelayError::Ipc("agent RPC is not supported on this platform".to_string()).into())
}

#[cfg(unix)]
fn agent_rpc_error(socket: &Path, err: DevRelayError) -> DevRelayError {
    match err {
        DevRelayError::Ipc(detail) => DevRelayError::Ipc(detail),
        other => DevRelayError::Ipc(format!(
            "failed to contact local DevRelay agent at {}: {other}; pass --direct to bypass the agent",
            socket.display()
        )),
    }
}

fn render_status_result(
    project_name: &str,
    result: &StatusGetResult,
    json: bool,
) -> anyhow::Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "project": project_name,
                "status": result.status,
                "untracked_policy": result.untracked,
            }))?
        );
    } else {
        println!(
            "{} / {}",
            project_name,
            result.status.branch.as_deref().unwrap_or("detached")
        );
        println!("  head: {}", result.status.head_oid);
        println!("  state: {}", status_summary_label(&result.status));
        let included = result
            .untracked
            .iter()
            .filter(|item| item.decision == PathDecision::Include)
            .count();
        let excluded = result.untracked.len() - included;
        println!("  untracked policy: {included} included, {excluded} excluded");
        if included > 0 {
            println!("  included untracked:");
            for item in result
                .untracked
                .iter()
                .filter(|item| item.decision == PathDecision::Include)
            {
                println!("    + {} ({})", item.path, item.reason);
            }
        }
        if excluded > 0 {
            println!("  excluded untracked:");
            for item in result
                .untracked
                .iter()
                .filter(|item| item.decision == PathDecision::Exclude)
            {
                println!("    - {} ({})", item.path, item.reason);
            }
        }
    }
    Ok(())
}

fn status_summary_label(status: &StatusSummary) -> String {
    format!(
        "{} staged, {} unstaged, {} untracked, {} unmerged",
        status.counts.staged,
        status.counts.unstaged,
        status.counts.untracked,
        status.counts.unmerged
    )
}

fn absolute_cli_path(path: PathBuf) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn ensure_workspace_not_registered(config: &LocalConfig, local_path: &Path) -> anyhow::Result<()> {
    if let Some((project, workspace)) = config.project_registry.workspace_by_path(local_path) {
        return Err(DevRelayError::Config(format!(
            "{} is already registered as workspace {} for {}",
            local_path.display(),
            workspace.workspace_id,
            project.project_id
        ))
        .into());
    }
    for project in config.project_registry.projects.values() {
        if project.workspaces.is_empty() && project.local_path == local_path {
            return Err(DevRelayError::Config(format!(
                "{} is already registered as {}",
                local_path.display(),
                project.project_id
            ))
            .into());
        }
    }
    Ok(())
}

fn refresh_workspace_states(config: &mut LocalConfig) -> bool {
    let mut changed = false;
    for project in config.project_registry.projects.values_mut() {
        for workspace in project.workspaces.values_mut() {
            let next_state = if workspace.local_path.exists() {
                match workspace.state {
                    WorkspaceState::Stale => WorkspaceState::Active,
                    current => current,
                }
            } else {
                WorkspaceState::Stale
            };
            if workspace.state != next_state {
                workspace.state = next_state;
                changed = true;
            }
        }
    }
    changed
}

fn remove_workspace(
    config: &mut LocalConfig,
    id_or_path: &str,
) -> anyhow::Result<(String, WorkspaceRegistryEntry)> {
    let (project_id, workspace_id) = find_workspace_ids(config, id_or_path)?;
    let project = config
        .project_registry
        .projects
        .get_mut(&project_id)
        .ok_or_else(|| DevRelayError::Config("workspace project disappeared".to_string()))?;
    let removed = project
        .workspaces
        .remove(&workspace_id)
        .ok_or_else(|| DevRelayError::Config("workspace disappeared".to_string()))?;
    if project.local_path == removed.local_path
        && let Some(next_workspace) = project.workspaces.values().next()
    {
        project.local_path = next_workspace.local_path.clone();
    }
    Ok((project_id, removed))
}

fn find_workspace_ids(config: &LocalConfig, id_or_path: &str) -> anyhow::Result<(String, String)> {
    if let Some((project, workspace)) = config.project_registry.workspace_by_id(id_or_path) {
        return Ok((project.project_id.clone(), workspace.workspace_id.clone()));
    }

    let lookup_paths = workspace_lookup_paths(id_or_path);
    for project in config.project_registry.projects.values() {
        for workspace in project.workspaces.values() {
            if lookup_paths
                .iter()
                .any(|path| path == &workspace.local_path)
            {
                return Ok((project.project_id.clone(), workspace.workspace_id.clone()));
            }
        }
    }

    Err(DevRelayError::Config(format!("unknown workspace {id_or_path}")).into())
}

fn workspace_lookup_paths(id_or_path: &str) -> Vec<PathBuf> {
    let raw = PathBuf::from(id_or_path);
    let mut paths = vec![raw.clone()];
    if raw.exists() {
        if let Ok(root) = resolve_git_root(&raw) {
            paths.push(root);
        } else if let Ok(canonical) = raw.canonicalize() {
            paths.push(canonical);
        }
    }
    paths
}

fn recover_list_snapshots(
    home: &DevRelayHome,
    config: &LocalConfig,
    project: Option<&str>,
) -> anyhow::Result<Vec<StoredSnapshot>> {
    if let Some(project) = project {
        let entry = find_project(config, project)?;
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

fn find_recovery_snapshot(
    home: &DevRelayHome,
    config: &LocalConfig,
    project: Option<&str>,
    snapshot_id: &str,
) -> anyhow::Result<(ProjectRegistryEntry, SnapshotStore, StoredSnapshot)> {
    if let Some(project) = project {
        let entry = find_project(config, project)?.clone();
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

    Err(DevRelayError::Recover(format!("unknown snapshot {snapshot_id}")).into())
}

fn recovery_source_path(project: &ProjectRegistryEntry) -> anyhow::Result<PathBuf> {
    if let Some(workspace) = project.workspaces.values().find(|workspace| {
        workspace.local_path.exists() && workspace.state == WorkspaceState::Active
    }) {
        return Ok(workspace.local_path.clone());
    }
    if project.local_path.exists() {
        return Ok(project.local_path.clone());
    }
    Err(DevRelayError::Recover(format!(
        "no existing source workspace for project {}",
        project.project_id
    ))
    .into())
}

fn prepare_recovery_workspace(path: &Path, source_path: &Path) -> anyhow::Result<GitRepo> {
    if path.join(".git").exists() {
        let target = GitRepo::new(path);
        let status = target.status()?;
        if !status.is_clean() {
            return Err(DevRelayError::TargetDirty(status.short_summary()).into());
        }
        return Ok(target);
    }

    if path.exists() && fs::read_dir(path)?.next().is_some() {
        return Err(DevRelayError::Config(format!(
            "{} exists and is not an empty recovery directory",
            path.display()
        ))
        .into());
    }

    fs::create_dir_all(path)?;
    clone_repository(source_path, path)?;
    Ok(GitRepo::new(path))
}

fn clone_repository(source_path: &Path, target_path: &Path) -> anyhow::Result<()> {
    let output = std::process::Command::new("git")
        .arg("clone")
        .arg(source_path)
        .arg(target_path)
        .output()?;
    if !output.status.success() {
        return Err(DevRelayError::GitCommand {
            cwd: source_path.to_path_buf(),
            args: format!("clone {} {}", source_path.display(), target_path.display()),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        }
        .into());
    }
    Ok(())
}

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
        .ok_or_else(|| DevRelayError::Config(format!("unknown project {project_id}")))?;
    project.workspaces.insert(workspace_id, workspace.clone());
    Ok(workspace)
}

fn mark_handoff_workspace_states(
    config: &mut LocalConfig,
    source_path: &Path,
    target_path: &Path,
) -> bool {
    let mut changed = false;
    for project in config.project_registry.projects.values_mut() {
        for workspace in project.workspaces.values_mut() {
            let next = if workspace.local_path == source_path {
                Some(WorkspaceState::Inactive)
            } else if workspace.local_path == target_path {
                Some(WorkspaceState::Active)
            } else {
                None
            };
            if let Some(next) = next
                && workspace.state != next
            {
                workspace.state = next;
                changed = true;
            }
        }
    }
    changed
}

struct PreparedApplyTarget {
    repo: GitRepo,
    backup: Option<StoredSnapshot>,
    safe_actions: Vec<String>,
}

fn prepare_apply_target(
    target: &GitRepo,
    snapshot: &SnapshotMetadata,
    dirty_policy: DirtyPolicy,
) -> anyhow::Result<PreparedApplyTarget> {
    let status = target.status()?;
    if status.is_clean() {
        return Ok(PreparedApplyTarget {
            repo: target.clone(),
            backup: None,
            safe_actions: Vec::new(),
        });
    }

    match dirty_policy {
        DirtyPolicy::Block => Err(DevRelayError::TargetDirty(status.short_summary()).into()),
        DirtyPolicy::SnapshotAndFork => {
            let backup = backup_dirty_target(target, snapshot)?;
            reset_to_clean_worktree(target)?;
            Ok(PreparedApplyTarget {
                repo: target.clone(),
                safe_actions: vec![format!(
                    "separate work preserved as pinned backup snapshot {}",
                    backup.snapshot_id
                )],
                backup: Some(backup),
            })
        }
        DirtyPolicy::NewWorkspace => {
            let recovery_path = next_new_workspace_path(target.path(), &snapshot.snapshot_id);
            clone_repository(target.path(), &recovery_path)?;
            Ok(PreparedApplyTarget {
                repo: GitRepo::new(&recovery_path),
                backup: None,
                safe_actions: vec![format!(
                    "dirty target left unchanged; applying in {}",
                    recovery_path.display()
                )],
            })
        }
    }
}

fn backup_dirty_target(
    target: &GitRepo,
    source_snapshot: &SnapshotMetadata,
) -> anyhow::Result<StoredSnapshot> {
    let backup_manifest = backup_manifest_from_snapshot(source_snapshot);
    let mut backup = create_snapshot(target, &backup_manifest)?;
    backup.session_id = Some(format!(
        "fork_{}",
        hash_text(&format!(
            "{}:{}:{}",
            source_snapshot.snapshot_id,
            backup.snapshot_id,
            target.path().display()
        ))
    ));
    let home = DevRelayHome::resolve()?;
    home.create_base_dirs()?;
    let mut store = SnapshotStore::open(&home, &source_snapshot.project_id)?;
    let label = Some(format!(
        "dirty target backup before {}",
        source_snapshot.snapshot_id
    ));
    Ok(store.store_snapshot(target, backup, true, label)?)
}

fn backup_manifest_from_snapshot(snapshot: &SnapshotMetadata) -> Manifest {
    Manifest {
        schema: 1,
        project_id: snapshot.project_id.clone(),
        name: snapshot.project_name.clone(),
        workspace: WorkspaceConfig {
            untracked: UntrackedPolicy::AllNonignored,
            portable_paths: PortablePathsPolicy::Strict,
            large_file_threshold_mib: 32,
            preserve_editor_context: true,
            preserve_unsaved_buffers: true,
            exclude: PatternConfig::default(),
            include: PatternConfig::default(),
            secret_scanner: SecretScannerConfig::default(),
        },
        environment: None,
        secrets: BTreeMap::new(),
        tasks: BTreeMap::new(),
        sync: None,
        handoff: None,
    }
}

fn reset_to_clean_worktree(target: &GitRepo) -> anyhow::Result<()> {
    target.run(&["reset", "--hard"])?;
    target.run(&["clean", "-fd"])?;
    let status = target.status()?;
    if !status.is_clean() {
        return Err(DevRelayError::TargetDirty(status.short_summary()).into());
    }
    Ok(())
}

fn next_new_workspace_path(target_path: &Path, snapshot_id: &str) -> PathBuf {
    let parent = target_path.parent().unwrap_or_else(|| Path::new("."));
    let base = target_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "workspace".to_string());
    let short_id = snapshot_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(10)
        .collect::<String>();
    for index in 0..100 {
        let suffix = if index == 0 {
            String::new()
        } else {
            format!("-{index}")
        };
        let candidate = parent.join(format!("{base}-devrelay-{short_id}{suffix}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{base}-devrelay-{}", hash_text(snapshot_id)))
}

fn find_project<'a>(
    config: &'a LocalConfig,
    id_or_name: &str,
) -> anyhow::Result<&'a devrelay_core::ProjectRegistryEntry> {
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
        .ok_or_else(|| DevRelayError::Config(format!("unknown project {id_or_name}")).into())
}

fn generated_project_id(root: &Path) -> String {
    format!("p_{}", hash_text(&root.to_string_lossy()))
}

fn remote_fingerprint(repo: &GitRepo) -> Option<String> {
    repo.run(&["remote", "get-url", "origin"])
        .ok()
        .map(|remote| format!("remote_{}", hash_text(remote.trim())))
}

fn root_commit_fingerprint(repo: &GitRepo) -> Option<String> {
    repo.run(&["rev-list", "--max-parents=0", "HEAD"])
        .ok()
        .and_then(|roots| roots.lines().next().map(str::to_string))
        .map(|root| format!("root_{}", hash_text(&root)))
}

fn head_oid(repo: &GitRepo) -> Option<String> {
    repo.run(&["rev-parse", "--verify", "HEAD"])
        .ok()
        .map(|head| head.trim().to_string())
        .filter(|head| !head.is_empty())
}

fn current_platform_profile() -> String {
    current_platform_key()
}

fn workspace_state_label(state: WorkspaceState) -> &'static str {
    match state {
        WorkspaceState::Active => "active",
        WorkspaceState::Inactive => "inactive",
        WorkspaceState::Stale => "stale",
    }
}

fn hash_text(value: &str) -> String {
    let digest = blake3::hash(value.as_bytes());
    digest.to_hex()[..16].to_string()
}

fn render_error(err: &Error, json: bool) {
    let info = error_info(err);
    let diagnostic_id = diagnostic_id(info.code, &info.detail);
    if json {
        let rendered = serde_json::json!({
            "error": {
                "code": info.code,
                "title": info.title,
                "message": err.to_string(),
                "detail": info.detail,
                "safe_actions": info.safe_actions,
                "diagnostic_id": diagnostic_id,
            }
        });
        eprintln!(
            "{}",
            serde_json::to_string_pretty(&rendered).unwrap_or_else(|_| {
                format!(
                    r#"{{"error":{{"code":"{}","title":"{}","message":"{}","detail":"{}","safe_actions":[],"diagnostic_id":"{}"}}}}"#,
                    info.code, info.title, err, info.detail, diagnostic_id
                )
            })
        );
    } else {
        eprintln!("error[{}]: {}", info.code, info.title);
        eprintln!("  detail: {}", info.detail);
        eprintln!("  diagnostic: {diagnostic_id}");
        if !info.safe_actions.is_empty() {
            eprintln!("  safe actions:");
            for action in info.safe_actions {
                eprintln!("    - {action}");
            }
        }
    }
}

fn error_info(err: &Error) -> ErrorInfo {
    for cause in err.chain() {
        if let Some(devrelay) = cause.downcast_ref::<DevRelayError>() {
            return devrelay.info();
        }
    }
    ErrorInfo {
        code: "DR-CLI-ERROR",
        title: "CLI error",
        detail: err.to_string(),
        safe_actions: vec!["Retry with --json-errors and inspect the full command output."],
    }
}

fn diagnostic_id(code: &str, detail: &str) -> String {
    format!("diag_{}", hash_text(&format!("{code}:{detail}")))
}
