//! Command-line interface for the local DevRelay foundation.
//!
//! This binary is intentionally thin in M0. It loads manifests, delegates Git
//! state work to `devrelay-core`, and renders human or JSON output for explicit
//! local commands.

use anyhow::{Context, Error};
use clap::{Parser, Subcommand};
use devrelay_core::{
    DevRelayError, DevRelayHome, GitRepo, LocalConfig, Manifest, PathDecision, apply_snapshot,
    classify_untracked_paths, create_snapshot, plan_apply_snapshot, read_snapshot_file,
    write_snapshot_file,
};
use std::path::PathBuf;
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
    Manifest {
        #[command(subcommand)]
        command: ManifestCommand,
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
        #[arg(long)]
        json: bool,
    },
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
            json,
        } => {
            let manifest = Manifest::load(&manifest)
                .with_context(|| format!("failed to load {}", manifest.display()))?;
            let repo = GitRepo::new(repo);
            let snapshot = create_snapshot(&repo, &manifest)?;
            let out = out.unwrap_or_else(|| {
                PathBuf::from(".devrelay")
                    .join("snapshots")
                    .join(format!("{}.json", snapshot.snapshot_id))
            });
            write_snapshot_file(&out, &snapshot)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&snapshot)?);
            } else {
                println!("checkpoint: {}", snapshot.snapshot_id);
                println!("  head: {}", snapshot.head_oid);
                println!("  index: {}", snapshot.index_tree_oid);
                println!("  work: {}", snapshot.work_tree_oid);
                println!(
                    "  included untracked: {}",
                    snapshot.included_untracked.len()
                );
                println!("  snapshot file: {}", out.display());
            }
        }
        Command::Apply {
            repo,
            source,
            snapshot,
            dry_run,
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
                let verification = apply_snapshot(&target, &source, &snapshot)?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "applied": snapshot.snapshot_id,
                            "verification": verification,
                        }))?
                    );
                } else {
                    println!("applied: {}", snapshot.snapshot_id);
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

fn render_error(err: &Error, json: bool) {
    let code = error_code(err);
    if json {
        let rendered = serde_json::json!({
            "error": {
                "code": code,
                "message": err.to_string(),
            }
        });
        eprintln!(
            "{}",
            serde_json::to_string_pretty(&rendered).unwrap_or_else(|_| {
                format!(r#"{{"error":{{"code":"{code}","message":"{}"}}}}"#, err)
            })
        );
    } else {
        eprintln!("error[{code}]: {err}");
    }
}

fn error_code(err: &Error) -> &'static str {
    for cause in err.chain() {
        if let Some(devrelay) = cause.downcast_ref::<DevRelayError>() {
            return devrelay.code();
        }
    }
    "DR-CLI-ERROR"
}
