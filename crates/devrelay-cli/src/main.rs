//! Command-line interface for the local DevRelay foundation.
//!
//! This binary is intentionally thin in M0. It loads manifests, delegates Git
//! state work to `devrelay-core`, and renders human or JSON output for explicit
//! local commands.

use anyhow::Context;
use clap::{Parser, Subcommand};
use devrelay_core::{
    GitRepo, Manifest, PathDecision, apply_snapshot, classify_untracked_paths, create_snapshot,
    read_snapshot_file, write_snapshot_file,
};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "devrelay")]
#[command(about = "DevRelay personal development fabric CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
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
    },
}

#[derive(Debug, Subcommand)]
enum ManifestCommand {
    Check { path: PathBuf },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Manifest { command } => match command {
            ManifestCommand::Check { path } => {
                let manifest = Manifest::load(&path)
                    .with_context(|| format!("failed to load {}", path.display()))?;
                println!(
                    "ok: {} ({}) schema {}",
                    manifest.name, manifest.project_id, manifest.schema
                );
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
        } => {
            let snapshot = read_snapshot_file(&snapshot)
                .with_context(|| format!("failed to read {}", snapshot.display()))?;
            let target = GitRepo::new(repo);
            let source = GitRepo::new(source);
            apply_snapshot(&target, &source, &snapshot)?;
            println!("applied: {}", snapshot.snapshot_id);
        }
    }
    Ok(())
}
