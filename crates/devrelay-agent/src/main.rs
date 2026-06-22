use anyhow::Context;
use clap::{Parser, ValueEnum};
use devrelay_core::{DevRelayHome, LocalConfig, MetadataDb};
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
        .unwrap_or_else(|| home.root().join("agent.sock"));

    eprintln!(
        "devrelay-agent started foreground={} log_level={:?} projects={} socket={}",
        cli.foreground,
        cli.log_level,
        config.project_registry.projects.len(),
        socket_path.display()
    );

    if cli.health {
        let health = AgentHealth {
            status: "ok",
            foreground: cli.foreground,
            config_path,
            socket_path,
            project_count: config.project_registry.projects.len(),
            database_path,
            shutdown_requested: shutdown.load(Ordering::SeqCst),
        };
        println!("{}", serde_json::to_string_pretty(&health)?);
        return Ok(());
    }

    if cli.foreground {
        while !shutdown.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(100));
        }
        eprintln!("devrelay-agent shutdown requested");
    }
    Ok(())
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
