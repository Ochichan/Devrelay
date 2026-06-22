//! Command-line interface for the local DevRelay foundation.
//!
//! This binary is intentionally thin in M0. It loads manifests, delegates Git
//! state work to `devrelay-core`, and renders human or JSON output for explicit
//! local commands.

use anyhow::{Context, Error};
use clap::{Parser, Subcommand, ValueEnum};
use devrelay_core::{
    DevRelayError, DevRelayHome, ErrorInfo, GitRepo, LocalConfig, Manifest, PathDecision,
    PatternConfig, PortablePathsPolicy, ProjectRegistryEntry, SnapshotMetadata, SnapshotStore,
    StoredSnapshot, UntrackedPolicy, WorkspaceConfig, WorkspaceRegistryEntry, WorkspaceState,
    apply_snapshot, classify_untracked_paths, create_snapshot, plan_apply_snapshot,
    read_snapshot_file, workspace_id_for, write_snapshot_file,
};
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
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
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
    match cli.command {
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
                let config = LocalConfig::default();
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
        Command::Project { command } => match command {
            ProjectCommand::Add {
                path,
                manifest,
                config,
                json,
            } => {
                let (config_path, mut local_config) = load_or_default_config(config)?;
                refresh_workspace_states(&mut local_config);
                let entry = build_project_registry_entry(
                    &path,
                    manifest.as_deref(),
                    &local_config.device_id,
                )?;
                ensure_workspace_not_registered(&local_config, &entry.local_path)?;
                let project_id = entry.project_id.clone();
                merge_project_registry_entry(&mut local_config, entry);
                let added = local_config
                    .project_registry
                    .projects
                    .get(&project_id)
                    .cloned()
                    .ok_or_else(|| DevRelayError::Config("project disappeared".to_string()))?;
                local_config.save(&config_path)?;
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
            }
            ProjectCommand::Show {
                id_or_name,
                config,
                json,
            } => {
                let (config_path, mut local_config) = load_or_default_config(config)?;
                if refresh_workspace_states(&mut local_config) {
                    local_config.save(&config_path)?;
                }
                let entry = find_project(&local_config, &id_or_name)?;
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
            }
            ProjectCommand::Remove {
                id_or_name,
                config,
                json,
            } => {
                let (config_path, mut local_config) = load_or_default_config(config)?;
                let project_id = find_project(&local_config, &id_or_name)?.project_id.clone();
                let removed = local_config
                    .project_registry
                    .projects
                    .remove(&project_id)
                    .ok_or_else(|| DevRelayError::Config("project disappeared".to_string()))?;
                local_config.save(&config_path)?;
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
            }
        },
        Command::Projects { command } => match command {
            ProjectsCommand::List { config, json } => {
                let (config_path, mut local_config) = load_or_default_config(config)?;
                if refresh_workspace_states(&mut local_config) {
                    local_config.save(&config_path)?;
                }
                let projects = local_config
                    .project_registry
                    .projects
                    .values()
                    .collect::<Vec<_>>();
                if json {
                    println!("{}", serde_json::to_string_pretty(&projects)?);
                } else {
                    for project in projects {
                        println!("{} ({})", project.display_name, project.project_id);
                        println!("  path: {}", project.local_path.display());
                        println!("  workspaces: {}", project.workspaces.len());
                    }
                }
            }
        },
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
        Command::Recover { command } => match command {
            RecoverCommand::List {
                project,
                config,
                json,
            } => {
                let (_, local_config) = load_or_default_config(config)?;
                let home = DevRelayHome::resolve()?;
                let snapshots = recover_list_snapshots(&home, &local_config, project.as_deref())?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&snapshots)?);
                } else {
                    for snapshot in snapshots {
                        println!(
                            "{} {} #{}",
                            snapshot.project_id, snapshot.snapshot_id, snapshot.sequence_number
                        );
                    }
                }
            }
            RecoverCommand::Show {
                snapshot_id,
                project,
                config,
                json,
            } => {
                let (_, local_config) = load_or_default_config(config)?;
                let home = DevRelayHome::resolve()?;
                let (_, _, snapshot) =
                    find_recovery_snapshot(&home, &local_config, project.as_deref(), &snapshot_id)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&snapshot)?);
                } else {
                    println!(
                        "snapshot: {} #{}",
                        snapshot.snapshot_id, snapshot.sequence_number
                    );
                    println!("  project: {}", snapshot.project_id);
                    if let Some(label) = snapshot.label {
                        println!("  label: {label}");
                    }
                }
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
                let (config_path, mut local_config) = load_or_default_config(config)?;
                let home = DevRelayHome::resolve()?;
                let (project_entry, store, snapshot) =
                    find_recovery_snapshot(&home, &local_config, project.as_deref(), &snapshot_id)?;
                let source_path = recovery_source_path(&project_entry)?;
                let target = prepare_recovery_workspace(&path, &source_path)?;
                let snapshot_source = GitRepo::new(store.snapshot_repo_path());
                let verification = apply_snapshot(&target, &snapshot_source, &snapshot.metadata)?;
                let registered_workspace = if register {
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
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "recovered": snapshot,
                            "path": target.path(),
                            "name": name,
                            "registered": registered_workspace,
                            "verification": verification,
                        }))?
                    );
                } else {
                    println!("recovered snapshot: {}", snapshot.snapshot_id);
                    println!("  path: {}", target.path().display());
                    if let Some(name) = name {
                        println!("  name: {name}");
                    }
                    if registered_workspace.is_some() {
                        println!("  registered: true");
                    }
                }
            }
        },
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
        } => {
            let manifest = Manifest::load(&manifest)
                .with_context(|| format!("failed to load {}", manifest.display()))?;
            let repo = GitRepo::new(repo);
            let status = repo.status()?;
            let classified =
                classify_untracked_paths(repo.path(), &manifest, status.untracked_paths())?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "project": manifest.name,
                        "status": status.summary(),
                        "untracked_policy": classified,
                    }))?
                );
            } else {
                println!(
                    "{} / {}",
                    manifest.name,
                    status.branch.as_deref().unwrap_or("detached")
                );
                println!("  head: {}", status.head_oid);
                println!("  state: {}", status.short_summary());
                let included = classified
                    .iter()
                    .filter(|item| item.decision == PathDecision::Include)
                    .count();
                let excluded = classified.len() - included;
                println!("  untracked policy: {included} included, {excluded} excluded");
                if included > 0 {
                    println!("  included untracked:");
                    for item in classified
                        .iter()
                        .filter(|item| item.decision == PathDecision::Include)
                    {
                        println!("    + {} ({})", item.path, item.reason);
                    }
                }
                if excluded > 0 {
                    println!("  excluded untracked:");
                    for item in classified
                        .iter()
                        .filter(|item| item.decision == PathDecision::Exclude)
                    {
                        println!("    - {} ({})", item.path, item.reason);
                    }
                }
            }
        }
        Command::Checkpoint {
            repo,
            manifest,
            out,
            label,
            pin,
            json,
        } => {
            let manifest = Manifest::load(&manifest)
                .with_context(|| format!("failed to load {}", manifest.display()))?;
            let repo = GitRepo::new(repo);
            let home = DevRelayHome::resolve()?;
            home.create_base_dirs()?;
            let mut store = SnapshotStore::open(&home, &manifest.project_id)?;
            let stored = store.checkpoint(&repo, &manifest, pin, label)?;
            if let Some(out) = &out {
                write_snapshot_file(out, &stored.metadata)?;
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "checkpoint": stored,
                        "snapshot_file": out,
                        "snapshot_repo": store.snapshot_repo_path(),
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
                println!("  snapshot repo: {}", store.snapshot_repo_path().display());
                if let Some(out) = &out {
                    println!("  snapshot file: {}", out.display());
                }
            }
        }
        Command::Apply {
            repo,
            source,
            snapshot,
            dry_run,
            dirty_policy,
            json,
        } => {
            let snapshot = read_snapshot_file(&snapshot)
                .with_context(|| format!("failed to read {}", snapshot.display()))?;
            let target = GitRepo::new(repo);
            let source = GitRepo::new(source);
            if dry_run {
                let plan = plan_apply_snapshot(&target, &source, &snapshot)?;
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
            } else {
                let prepared = prepare_apply_target(&target, &snapshot, dirty_policy)?;
                let verification = apply_snapshot(&prepared.repo, &source, &snapshot)?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "applied": snapshot.snapshot_id,
                            "applied_repo": prepared.repo.path(),
                            "dirty_policy": dirty_policy.label(),
                            "backup": prepared.backup,
                            "safe_actions": prepared.safe_actions,
                            "verification": verification,
                        }))?
                    );
                } else {
                    println!("applied: {}", snapshot.snapshot_id);
                    println!("  repo: {}", prepared.repo.path().display());
                    for action in prepared.safe_actions {
                        println!("  {action}");
                    }
                }
            }
        }
    }
    Ok(())
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
        Ok((path, LocalConfig::default()))
    }
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
                    "dirty target preserved as pinned backup snapshot {}",
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
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
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
