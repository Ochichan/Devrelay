//! Environment profile selection policy.
//!
//! This module does not execute environment commands. It chooses the profile
//! that a runner should hydrate after checking platform targets, adapter
//! availability, and bootstrap trust state.

use crate::{DevRelayError, EnvironmentKind, Manifest, Result, current_platform_key};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentSelectionContext {
    pub platform_key: String,
    pub available_kinds: BTreeSet<EnvironmentKind>,
    pub trusted_command_scopes: BTreeSet<String>,
}

impl EnvironmentSelectionContext {
    pub fn current() -> Self {
        Self {
            platform_key: current_platform_key(),
            available_kinds: BTreeSet::from([EnvironmentKind::Manual]),
            trusted_command_scopes: BTreeSet::new(),
        }
    }

    pub fn with_platform_key(platform_key: impl Into<String>) -> Self {
        Self {
            platform_key: platform_key.into(),
            available_kinds: BTreeSet::from([EnvironmentKind::Manual]),
            trusted_command_scopes: BTreeSet::new(),
        }
    }

    pub fn with_available_kind(mut self, kind: EnvironmentKind) -> Self {
        self.available_kinds.insert(kind);
        self
    }

    pub fn with_trusted_command_scope(mut self, scope: impl Into<String>) -> Self {
        self.trusted_command_scopes.insert(scope.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentProfileSelection {
    pub profile_name: Option<String>,
    pub kind: Option<EnvironmentKind>,
    pub command_scope: Option<String>,
    pub explanation: Vec<String>,
}

pub fn environment_profile_command_scope(profile_name: &str) -> String {
    format!("environment.profile.{profile_name}")
}

pub fn select_environment_profile(
    manifest: &Manifest,
    context: &EnvironmentSelectionContext,
) -> EnvironmentProfileSelection {
    let Some(environment) = &manifest.environment else {
        return EnvironmentProfileSelection {
            profile_name: None,
            kind: None,
            command_scope: None,
            explanation: vec!["manifest has no environment profiles".to_string()],
        };
    };

    let profiles = &environment.profiles;
    let mut explanation = Vec::new();
    for kind in [
        EnvironmentKind::Nix,
        EnvironmentKind::Devcontainer,
        EnvironmentKind::Native,
        EnvironmentKind::Script,
        EnvironmentKind::Manual,
    ] {
        for (name, profile) in profiles.iter().filter(|(_, profile)| profile.kind == kind) {
            let scope = environment_profile_command_scope(name);
            if !profile_targets_platform(&profile.targets, &context.platform_key) {
                explanation.push(format!(
                    "skipped {name}: targets do not match {}",
                    context.platform_key
                ));
                continue;
            }
            if !kind_available(kind, &context.available_kinds) {
                explanation.push(format!("skipped {name}: {:?} adapter unavailable", kind));
                continue;
            }
            if kind == EnvironmentKind::Script && !context.trusted_command_scopes.contains(&scope) {
                explanation.push(format!("skipped {name}: bootstrap command is not trusted"));
                continue;
            }

            explanation.push(format!(
                "selected {name}: {:?} profile matches {}",
                kind, context.platform_key
            ));
            return EnvironmentProfileSelection {
                profile_name: Some(name.clone()),
                kind: Some(kind),
                command_scope: Some(scope),
                explanation,
            };
        }
    }

    if profiles.is_empty() {
        explanation.push("manifest environment has no profiles".to_string());
    } else {
        explanation.push(format!(
            "no environment profile matches {} with available adapters",
            context.platform_key
        ));
    }
    EnvironmentProfileSelection {
        profile_name: None,
        kind: None,
        command_scope: None,
        explanation,
    }
}

pub fn profile_targets_platform(targets: &[String], platform_key: &str) -> bool {
    targets
        .iter()
        .any(|target| target_matches_platform_key(target, platform_key))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentCommand {
    pub program: String,
    pub args: Vec<String>,
    pub timeout_seconds: Option<u64>,
}

impl EnvironmentCommand {
    pub fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            timeout_seconds: None,
        }
    }

    pub fn with_timeout_seconds(mut self, timeout_seconds: Option<u64>) -> Self {
        self.timeout_seconds = timeout_seconds;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentCommandOutput {
    pub status_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

impl EnvironmentCommandOutput {
    pub fn success(stdout: impl Into<String>) -> Self {
        Self {
            status_code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
            timed_out: false,
        }
    }

    pub fn failure(status_code: i32, stderr: impl Into<String>) -> Self {
        Self {
            status_code: Some(status_code),
            stdout: String::new(),
            stderr: stderr.into(),
            timed_out: false,
        }
    }

    pub fn timed_out(stderr: impl Into<String>) -> Self {
        Self {
            status_code: None,
            stdout: String::new(),
            stderr: stderr.into(),
            timed_out: true,
        }
    }

    pub fn succeeded(&self) -> bool {
        self.status_code == Some(0)
    }
}

pub trait EnvironmentCommandRunner {
    fn run(&self, cwd: &Path, command: &EnvironmentCommand) -> Result<EnvironmentCommandOutput>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemEnvironmentCommandRunner;

impl EnvironmentCommandRunner for SystemEnvironmentCommandRunner {
    fn run(&self, cwd: &Path, command: &EnvironmentCommand) -> Result<EnvironmentCommandOutput> {
        let mut child = Command::new(&command.program)
            .args(&command.args)
            .current_dir(cwd)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;
        let timed_out = if let Some(timeout_seconds) = command.timeout_seconds {
            let started = std::time::Instant::now();
            loop {
                if child.try_wait()?.is_some() {
                    break false;
                }
                if started.elapsed() >= std::time::Duration::from_secs(timeout_seconds.max(1)) {
                    child.kill()?;
                    break true;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        } else {
            false
        };
        let output = child.wait_with_output()?;
        Ok(EnvironmentCommandOutput {
            status_code: if timed_out {
                None
            } else {
                output.status.code()
            },
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            timed_out,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NixFlakeFiles {
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NixHealthState {
    Unavailable,
    NoFlake,
    ShellReady,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NixDevelopHealthcheck {
    pub state: NixHealthState,
    pub command: Vec<String>,
    pub shell_ready: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub failure_logs: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NixPlaceholderPlan {
    pub enabled: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NixCacheWarmth {
    pub platform_key: String,
    pub score: u8,
    pub explanation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NixAdapterReport {
    pub nix_available: bool,
    pub flake_files: NixFlakeFiles,
    pub flake_fingerprint: Option<String>,
    pub healthcheck: NixDevelopHealthcheck,
    pub store_prefetch: NixPlaceholderPlan,
    pub lan_binary_cache: NixPlaceholderPlan,
    pub cache_warmth: NixCacheWarmth,
}

pub fn inspect_nix_environment(
    root: &Path,
    healthcheck: &[String],
    platform_key: &str,
    runner: &impl EnvironmentCommandRunner,
) -> Result<NixAdapterReport> {
    let nix_available = detect_nix_availability(root, runner)?;
    let flake_files = detect_nix_flake_files(root);
    let flake_fingerprint = compute_nix_flake_fingerprint(root)?;
    let healthcheck = run_nix_develop_healthcheck(root, healthcheck, nix_available, runner)?;
    let cache_warmth =
        estimate_nix_cache_warmth(platform_key, nix_available, flake_fingerprint.as_deref());
    Ok(NixAdapterReport {
        nix_available,
        flake_files,
        flake_fingerprint,
        healthcheck,
        store_prefetch: nix_store_prefetch_plan(),
        lan_binary_cache: nix_lan_binary_cache_plan(),
        cache_warmth,
    })
}

pub fn detect_nix_availability(
    root: &Path,
    runner: &impl EnvironmentCommandRunner,
) -> Result<bool> {
    match runner.run(root, &EnvironmentCommand::new("nix", ["--version"])) {
        Ok(output) => Ok(output.succeeded()),
        Err(DevRelayError::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

pub fn detect_nix_flake_files(root: &Path) -> NixFlakeFiles {
    let files = ["flake.nix", "flake.lock"]
        .into_iter()
        .map(PathBuf::from)
        .filter(|relative| root.join(relative).is_file())
        .collect();
    NixFlakeFiles { files }
}

pub fn compute_nix_flake_fingerprint(root: &Path) -> Result<Option<String>> {
    let flake_files = detect_nix_flake_files(root);
    if flake_files.files.is_empty() {
        return Ok(None);
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"devrelay.nix-flake.v1\0");
    for relative in &flake_files.files {
        let bytes = fs::read(root.join(relative))?;
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update(&[0]);
        hasher.update(bytes.len().to_string().as_bytes());
        hasher.update(&[0]);
        hasher.update(&bytes);
        hasher.update(&[0]);
    }
    Ok(Some(hasher.finalize().to_hex().to_string()))
}

pub fn run_nix_develop_healthcheck(
    root: &Path,
    healthcheck: &[String],
    nix_available: bool,
    runner: &impl EnvironmentCommandRunner,
) -> Result<NixDevelopHealthcheck> {
    let command = nix_develop_command(healthcheck);
    if !nix_available {
        return Ok(NixDevelopHealthcheck {
            state: NixHealthState::Unavailable,
            command,
            shell_ready: false,
            exit_code: None,
            stdout: String::new(),
            stderr: "nix command is not available".to_string(),
            failure_logs: "nix command is not available".to_string(),
        });
    }
    if !root.join("flake.nix").is_file() {
        return Ok(NixDevelopHealthcheck {
            state: NixHealthState::NoFlake,
            command,
            shell_ready: false,
            exit_code: None,
            stdout: String::new(),
            stderr: "flake.nix was not found".to_string(),
            failure_logs: "flake.nix was not found".to_string(),
        });
    }

    let output = runner.run(root, &EnvironmentCommand::new("nix", command.clone()))?;
    let shell_ready = output.succeeded();
    Ok(NixDevelopHealthcheck {
        state: if shell_ready {
            NixHealthState::ShellReady
        } else {
            NixHealthState::Failed
        },
        command,
        shell_ready,
        exit_code: output.status_code,
        failure_logs: if shell_ready {
            String::new()
        } else {
            join_logs(&output.stdout, &output.stderr)
        },
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

pub fn nix_store_prefetch_plan() -> NixPlaceholderPlan {
    NixPlaceholderPlan {
        enabled: false,
        reason: "store path prefetch is reserved for a future Nix store transfer implementation"
            .to_string(),
    }
}

pub fn nix_lan_binary_cache_plan() -> NixPlaceholderPlan {
    NixPlaceholderPlan {
        enabled: false,
        reason: "LAN binary cache configuration is reserved for anchor-backed cache distribution"
            .to_string(),
    }
}

pub fn estimate_nix_cache_warmth(
    platform_key: &str,
    nix_available: bool,
    flake_fingerprint: Option<&str>,
) -> NixCacheWarmth {
    let (score, explanation) = if !nix_available {
        (0, "nix is unavailable on this platform".to_string())
    } else if flake_fingerprint.is_some() {
        (
            50,
            format!(
                "{platform_key} has nix and a fingerprinted flake, but no store-path probe yet"
            ),
        )
    } else {
        (
            10,
            format!("{platform_key} has nix, but no flake fingerprint is available"),
        )
    };
    NixCacheWarmth {
        platform_key: platform_key.to_string(),
        score,
        explanation,
    }
}

fn nix_develop_command(healthcheck: &[String]) -> Vec<String> {
    let mut command = vec!["develop".to_string(), "--command".to_string()];
    if healthcheck.is_empty() {
        command.push("true".to_string());
    } else {
        command.extend(healthcheck.iter().cloned());
    }
    command
}

fn join_logs(stdout: &str, stderr: &str) -> String {
    [stdout.trim(), stderr.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerEngine {
    Docker,
    Podman,
}

impl ContainerEngine {
    pub const fn command(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevContainerPrepareState {
    NoConfig,
    EngineUnavailable,
    ApprovalRequired,
    Prepared,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevContainerHealthState {
    NotReady,
    ShellReady,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevContainerMountPlan {
    pub source: PathBuf,
    pub target: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevContainerImagePrepare {
    pub state: DevContainerPrepareState,
    pub command: Vec<String>,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub logs: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevContainerHealthcheck {
    pub state: DevContainerHealthState,
    pub command: Vec<String>,
    pub shell_ready: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub failure_logs: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevContainerAdapterReport {
    pub config_path: Option<PathBuf>,
    pub engine: Option<ContainerEngine>,
    pub fingerprint: Option<String>,
    pub mount_plan: Option<DevContainerMountPlan>,
    pub image_prepare: DevContainerImagePrepare,
    pub healthcheck: DevContainerHealthcheck,
}

pub fn inspect_devcontainer_environment(
    root: &Path,
    healthcheck: &[String],
    allow_image_pull_or_build: bool,
    runner: &impl EnvironmentCommandRunner,
) -> Result<DevContainerAdapterReport> {
    let config_path = detect_devcontainer_config(root);
    let engine = detect_container_engine(root, runner)?;
    let fingerprint = compute_devcontainer_fingerprint(root)?;
    let mount_plan = config_path.as_ref().map(|_| devcontainer_mount_plan(root));
    let image_prepare = prepare_devcontainer_image(
        root,
        config_path.is_some(),
        engine,
        allow_image_pull_or_build,
        runner,
    )?;
    let healthcheck = run_devcontainer_healthcheck(root, healthcheck, image_prepare.state, runner)?;
    Ok(DevContainerAdapterReport {
        config_path,
        engine,
        fingerprint,
        mount_plan,
        image_prepare,
        healthcheck,
    })
}

pub fn detect_devcontainer_config(root: &Path) -> Option<PathBuf> {
    let relative = PathBuf::from(".devcontainer/devcontainer.json");
    root.join(&relative).is_file().then_some(relative)
}

pub fn detect_container_engine(
    root: &Path,
    runner: &impl EnvironmentCommandRunner,
) -> Result<Option<ContainerEngine>> {
    for engine in [ContainerEngine::Docker, ContainerEngine::Podman] {
        match runner.run(
            root,
            &EnvironmentCommand::new(engine.command(), ["--version"]),
        ) {
            Ok(output) if output.succeeded() => return Ok(Some(engine)),
            Ok(_) => {}
            Err(DevRelayError::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(None)
}

pub fn compute_devcontainer_fingerprint(root: &Path) -> Result<Option<String>> {
    let files = devcontainer_fingerprint_files(root);
    if files.is_empty() {
        return Ok(None);
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"devrelay.devcontainer.v1\0");
    for relative in files {
        let bytes = fs::read(root.join(&relative))?;
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update(&[0]);
        hasher.update(bytes.len().to_string().as_bytes());
        hasher.update(&[0]);
        hasher.update(&bytes);
        hasher.update(&[0]);
    }
    Ok(Some(hasher.finalize().to_hex().to_string()))
}

pub fn devcontainer_mount_plan(root: &Path) -> DevContainerMountPlan {
    let name = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace");
    DevContainerMountPlan {
        source: root.to_path_buf(),
        target: format!("/workspaces/{name}"),
        read_only: false,
    }
}

pub fn prepare_devcontainer_image(
    root: &Path,
    config_present: bool,
    engine: Option<ContainerEngine>,
    allow_image_pull_or_build: bool,
    runner: &impl EnvironmentCommandRunner,
) -> Result<DevContainerImagePrepare> {
    let command = devcontainer_up_command(root);
    if !config_present {
        return Ok(devcontainer_prepare_terminal(
            DevContainerPrepareState::NoConfig,
            command,
            None,
            "",
            "devcontainer.json was not found",
        ));
    }
    if engine.is_none() {
        return Ok(devcontainer_prepare_terminal(
            DevContainerPrepareState::EngineUnavailable,
            command,
            None,
            "",
            "no supported container engine was found",
        ));
    }
    if !allow_image_pull_or_build {
        return Ok(devcontainer_prepare_terminal(
            DevContainerPrepareState::ApprovalRequired,
            command,
            None,
            "",
            "image pull/build requires user approval",
        ));
    }

    let output = runner.run(
        root,
        &EnvironmentCommand::new("devcontainer", command.clone()),
    )?;
    let prepared = output.succeeded();
    Ok(DevContainerImagePrepare {
        state: if prepared {
            DevContainerPrepareState::Prepared
        } else {
            DevContainerPrepareState::Failed
        },
        command,
        exit_code: output.status_code,
        logs: if prepared {
            String::new()
        } else {
            join_logs(&output.stdout, &output.stderr)
        },
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

pub fn run_devcontainer_healthcheck(
    root: &Path,
    healthcheck: &[String],
    prepare_state: DevContainerPrepareState,
    runner: &impl EnvironmentCommandRunner,
) -> Result<DevContainerHealthcheck> {
    let command = devcontainer_exec_command(root, healthcheck);
    if prepare_state != DevContainerPrepareState::Prepared {
        return Ok(DevContainerHealthcheck {
            state: DevContainerHealthState::NotReady,
            command,
            shell_ready: false,
            exit_code: None,
            stdout: String::new(),
            stderr: "devcontainer image is not prepared".to_string(),
            failure_logs: "devcontainer image is not prepared".to_string(),
        });
    }

    let output = runner.run(
        root,
        &EnvironmentCommand::new("devcontainer", command.clone()),
    )?;
    let shell_ready = output.succeeded();
    Ok(DevContainerHealthcheck {
        state: if shell_ready {
            DevContainerHealthState::ShellReady
        } else {
            DevContainerHealthState::Failed
        },
        command,
        shell_ready,
        exit_code: output.status_code,
        failure_logs: if shell_ready {
            String::new()
        } else {
            join_logs(&output.stdout, &output.stderr)
        },
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

fn devcontainer_fingerprint_files(root: &Path) -> Vec<PathBuf> {
    [
        ".devcontainer/devcontainer.json",
        ".devcontainer/Dockerfile",
        ".devcontainer/docker-compose.yml",
        ".devcontainer/compose.yml",
    ]
    .into_iter()
    .map(PathBuf::from)
    .filter(|relative| root.join(relative).is_file())
    .collect()
}

fn devcontainer_up_command(root: &Path) -> Vec<String> {
    vec![
        "up".to_string(),
        "--workspace-folder".to_string(),
        root.to_string_lossy().to_string(),
    ]
}

fn devcontainer_exec_command(root: &Path, healthcheck: &[String]) -> Vec<String> {
    let mut command = vec![
        "exec".to_string(),
        "--workspace-folder".to_string(),
        root.to_string_lossy().to_string(),
    ];
    if healthcheck.is_empty() {
        command.push("true".to_string());
    } else {
        command.extend(healthcheck.iter().cloned());
    }
    command
}

fn devcontainer_prepare_terminal(
    state: DevContainerPrepareState,
    command: Vec<String>,
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> DevContainerImagePrepare {
    DevContainerImagePrepare {
        state,
        command,
        exit_code,
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
        logs: stderr.to_string(),
    }
}

fn kind_available(kind: EnvironmentKind, available_kinds: &BTreeSet<EnvironmentKind>) -> bool {
    kind == EnvironmentKind::Manual || available_kinds.contains(&kind)
}

fn target_matches_platform_key(target: &str, platform_key: &str) -> bool {
    let target = target.trim();
    if target == "*" || target == "local" || target == platform_key {
        return true;
    }
    if let Some(prefix) = target.strip_suffix("-*") {
        return platform_key.starts_with(prefix);
    }
    platform_aliases(platform_key).contains(target)
}

fn platform_aliases(platform_key: &str) -> BTreeSet<&'static str> {
    if platform_key.starts_with("darwin-") {
        BTreeSet::from(["darwin", "macos"])
    } else if platform_key.starts_with("wsl2-linux-gnu-") {
        BTreeSet::from(["wsl2", "linux", "linux-gnu"])
    } else if platform_key.starts_with("linux-gnu-") {
        BTreeSet::from(["linux", "linux-gnu"])
    } else if platform_key.starts_with("windows-native-") {
        BTreeSet::from(["windows", "windows-native"])
    } else {
        BTreeSet::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EnvironmentConfig, EnvironmentProfile, UntrackedPolicy};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn manifest_with_profiles(
        profiles: impl IntoIterator<Item = (&'static str, EnvironmentKind, Vec<&'static str>)>,
    ) -> Manifest {
        let mut manifest = Manifest::parse(
            r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#,
        )
        .unwrap();
        let profiles = profiles
            .into_iter()
            .map(|(name, kind, targets)| {
                (
                    name.to_string(),
                    EnvironmentProfile {
                        kind,
                        targets: targets.into_iter().map(str::to_string).collect(),
                        command: vec!["echo".to_string(), name.to_string()],
                        fingerprint_files: Vec::new(),
                        healthcheck: None,
                        working_directory: None,
                        timeout_seconds: None,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        manifest.environment = Some(EnvironmentConfig { profiles });
        manifest
    }

    #[test]
    fn selection_context_detects_current_platform_key() {
        let context = EnvironmentSelectionContext::current();

        assert!(!context.platform_key.is_empty());
        assert!(context.available_kinds.contains(&EnvironmentKind::Manual));
    }

    #[test]
    fn matches_manifest_profile_targets_with_platform_aliases() {
        assert!(profile_targets_platform(
            &["darwin".to_string()],
            "darwin-arm64"
        ));
        assert!(profile_targets_platform(
            &["linux-gnu-*".to_string()],
            "linux-gnu-x86_64"
        ));
        assert!(profile_targets_platform(
            &["local".to_string()],
            "windows-native-x86_64"
        ));
        assert!(!profile_targets_platform(
            &["windows-native".to_string()],
            "darwin-arm64"
        ));
    }

    #[test]
    fn prefers_nix_then_devcontainer_then_native_then_trusted_script_then_manual() {
        let manifest = manifest_with_profiles([
            ("manual", EnvironmentKind::Manual, vec!["darwin"]),
            ("script", EnvironmentKind::Script, vec!["darwin"]),
            ("native", EnvironmentKind::Native, vec!["darwin"]),
            ("dev", EnvironmentKind::Devcontainer, vec!["darwin"]),
            ("nix", EnvironmentKind::Nix, vec!["darwin"]),
        ]);
        let script_scope = environment_profile_command_scope("script");

        let nix = select_environment_profile(
            &manifest,
            &EnvironmentSelectionContext::with_platform_key("darwin-arm64")
                .with_available_kind(EnvironmentKind::Nix)
                .with_available_kind(EnvironmentKind::Devcontainer)
                .with_available_kind(EnvironmentKind::Native)
                .with_available_kind(EnvironmentKind::Script)
                .with_trusted_command_scope(script_scope.clone()),
        );
        assert_eq!(nix.profile_name.as_deref(), Some("nix"));

        let devcontainer = select_environment_profile(
            &manifest,
            &EnvironmentSelectionContext::with_platform_key("darwin-arm64")
                .with_available_kind(EnvironmentKind::Devcontainer)
                .with_available_kind(EnvironmentKind::Native)
                .with_available_kind(EnvironmentKind::Script)
                .with_trusted_command_scope(script_scope.clone()),
        );
        assert_eq!(devcontainer.profile_name.as_deref(), Some("dev"));

        let native = select_environment_profile(
            &manifest,
            &EnvironmentSelectionContext::with_platform_key("darwin-arm64")
                .with_available_kind(EnvironmentKind::Native)
                .with_available_kind(EnvironmentKind::Script)
                .with_trusted_command_scope(script_scope.clone()),
        );
        assert_eq!(native.profile_name.as_deref(), Some("native"));

        let script = select_environment_profile(
            &manifest,
            &EnvironmentSelectionContext::with_platform_key("darwin-arm64")
                .with_available_kind(EnvironmentKind::Script)
                .with_trusted_command_scope(script_scope),
        );
        assert_eq!(script.profile_name.as_deref(), Some("script"));

        let manual = select_environment_profile(
            &manifest,
            &EnvironmentSelectionContext::with_platform_key("darwin-arm64")
                .with_available_kind(EnvironmentKind::Script),
        );
        assert_eq!(manual.profile_name.as_deref(), Some("manual"));
        assert!(
            manual
                .explanation
                .iter()
                .any(|line| line.contains("bootstrap command is not trusted"))
        );
    }

    #[test]
    fn returns_no_selection_when_no_target_matches() {
        let manifest =
            manifest_with_profiles([("manual", EnvironmentKind::Manual, vec!["windows-native"])]);

        let selected = select_environment_profile(
            &manifest,
            &EnvironmentSelectionContext::with_platform_key("darwin-arm64"),
        );

        assert_eq!(selected.profile_name, None);
        assert!(selected.explanation.iter().any(|line| {
            line.contains("targets do not match") || line.contains("no environment profile")
        }));
    }

    #[test]
    fn keeps_manifest_defaults_unchanged() {
        let manifest = Manifest::parse(
            r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#,
        )
        .unwrap();

        assert_eq!(manifest.workspace.untracked, UntrackedPolicy::Safe);
        assert_eq!(
            select_environment_profile(
                &manifest,
                &EnvironmentSelectionContext::with_platform_key("darwin-arm64"),
            )
            .profile_name,
            None
        );
    }

    #[derive(Default)]
    struct FakeRunner {
        outputs: BTreeMap<Vec<String>, EnvironmentCommandOutput>,
    }

    impl FakeRunner {
        fn with_output(
            mut self,
            program: &str,
            args: &[&str],
            output: EnvironmentCommandOutput,
        ) -> Self {
            let mut key = vec![program.to_string()];
            key.extend(args.iter().map(|arg| (*arg).to_string()));
            self.outputs.insert(key, output);
            self
        }
    }

    impl EnvironmentCommandRunner for FakeRunner {
        fn run(
            &self,
            _cwd: &Path,
            command: &EnvironmentCommand,
        ) -> Result<EnvironmentCommandOutput> {
            let mut key = vec![command.program.clone()];
            key.extend(command.args.iter().cloned());
            Ok(self
                .outputs
                .get(&key)
                .cloned()
                .unwrap_or_else(|| EnvironmentCommandOutput::failure(127, "command not found")))
        }
    }

    #[test]
    fn detects_flake_files_and_computes_stable_fingerprint() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("flake.nix"), "{ outputs = _: {}; }\n").unwrap();
        fs::write(temp.path().join("flake.lock"), "{}\n").unwrap();

        let files = detect_nix_flake_files(temp.path());
        assert_eq!(
            files.files,
            vec![PathBuf::from("flake.nix"), PathBuf::from("flake.lock")]
        );
        let first = compute_nix_flake_fingerprint(temp.path()).unwrap().unwrap();
        let second = compute_nix_flake_fingerprint(temp.path()).unwrap().unwrap();
        assert_eq!(first, second);

        fs::write(temp.path().join("flake.lock"), "{\"version\":1}\n").unwrap();
        let changed = compute_nix_flake_fingerprint(temp.path()).unwrap().unwrap();
        assert_ne!(first, changed);
    }

    #[test]
    fn nix_adapter_reports_shell_ready_with_mocked_healthcheck() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("flake.nix"), "{ outputs = _: {}; }\n").unwrap();
        let runner = FakeRunner::default()
            .with_output(
                "nix",
                &["--version"],
                EnvironmentCommandOutput::success("nix 2.21\n"),
            )
            .with_output(
                "nix",
                &["develop", "--command", "cargo", "check"],
                EnvironmentCommandOutput::success("checked\n"),
            );

        let report = inspect_nix_environment(
            temp.path(),
            &["cargo".to_string(), "check".to_string()],
            "darwin-arm64",
            &runner,
        )
        .unwrap();

        assert!(report.nix_available);
        assert!(report.flake_fingerprint.is_some());
        assert_eq!(report.healthcheck.state, NixHealthState::ShellReady);
        assert!(report.healthcheck.shell_ready);
        assert!(!report.store_prefetch.enabled);
        assert!(!report.lan_binary_cache.enabled);
        assert_eq!(report.cache_warmth.platform_key, "darwin-arm64");
        assert_eq!(report.cache_warmth.score, 50);
    }

    #[test]
    fn nix_adapter_captures_failure_logs() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("flake.nix"), "{ outputs = _: {}; }\n").unwrap();
        let runner = FakeRunner::default()
            .with_output(
                "nix",
                &["--version"],
                EnvironmentCommandOutput::success("nix 2.21\n"),
            )
            .with_output(
                "nix",
                &["develop", "--command", "cargo", "test"],
                EnvironmentCommandOutput {
                    status_code: Some(1),
                    stdout: "building\n".to_string(),
                    stderr: "tests failed\n".to_string(),
                },
            );

        let report = inspect_nix_environment(
            temp.path(),
            &["cargo".to_string(), "test".to_string()],
            "darwin-arm64",
            &runner,
        )
        .unwrap();

        assert_eq!(report.healthcheck.state, NixHealthState::Failed);
        assert!(!report.healthcheck.shell_ready);
        assert!(report.healthcheck.failure_logs.contains("building"));
        assert!(report.healthcheck.failure_logs.contains("tests failed"));
    }

    #[test]
    fn nix_adapter_reports_unavailable_and_missing_flake_states() {
        let temp = tempfile::tempdir().unwrap();
        let unavailable = inspect_nix_environment(
            temp.path(),
            &["true".to_string()],
            "darwin-arm64",
            &FakeRunner::default(),
        )
        .unwrap();
        assert!(!unavailable.nix_available);
        assert_eq!(unavailable.healthcheck.state, NixHealthState::Unavailable);
        assert_eq!(unavailable.cache_warmth.score, 0);

        let runner = FakeRunner::default().with_output(
            "nix",
            &["--version"],
            EnvironmentCommandOutput::success("nix 2.21\n"),
        );
        let no_flake = inspect_nix_environment(temp.path(), &[], "darwin-arm64", &runner).unwrap();
        assert!(no_flake.nix_available);
        assert_eq!(
            no_flake.healthcheck.command,
            vec!["develop", "--command", "true"]
        );
        assert_eq!(no_flake.healthcheck.state, NixHealthState::NoFlake);
    }

    #[test]
    fn devcontainer_detects_config_and_computes_fingerprint() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path().join(".devcontainer");
        fs::create_dir(&config_dir).unwrap();
        fs::write(
            config_dir.join("devcontainer.json"),
            r#"{"name":"demo","image":"ubuntu:24.04"}"#,
        )
        .unwrap();
        fs::write(config_dir.join("Dockerfile"), "FROM ubuntu:24.04\n").unwrap();

        assert_eq!(
            detect_devcontainer_config(temp.path()),
            Some(PathBuf::from(".devcontainer/devcontainer.json"))
        );
        let first = compute_devcontainer_fingerprint(temp.path())
            .unwrap()
            .unwrap();
        let second = compute_devcontainer_fingerprint(temp.path())
            .unwrap()
            .unwrap();
        assert_eq!(first, second);

        fs::write(config_dir.join("Dockerfile"), "FROM ubuntu:26.04\n").unwrap();
        let changed = compute_devcontainer_fingerprint(temp.path())
            .unwrap()
            .unwrap();
        assert_ne!(first, changed);
    }

    #[test]
    fn devcontainer_detects_container_engine_preference() {
        let temp = tempfile::tempdir().unwrap();
        let docker = FakeRunner::default().with_output(
            "docker",
            &["--version"],
            EnvironmentCommandOutput::success("Docker version 26\n"),
        );
        assert_eq!(
            detect_container_engine(temp.path(), &docker).unwrap(),
            Some(ContainerEngine::Docker)
        );

        let podman = FakeRunner::default()
            .with_output(
                "docker",
                &["--version"],
                EnvironmentCommandOutput::failure(127, "docker missing"),
            )
            .with_output(
                "podman",
                &["--version"],
                EnvironmentCommandOutput::success("podman version 5\n"),
            );
        assert_eq!(
            detect_container_engine(temp.path(), &podman).unwrap(),
            Some(ContainerEngine::Podman)
        );
    }

    #[test]
    fn devcontainer_requires_approval_before_image_prepare() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join(".devcontainer")).unwrap();
        fs::write(
            temp.path().join(".devcontainer/devcontainer.json"),
            r#"{"name":"demo","image":"ubuntu:24.04"}"#,
        )
        .unwrap();
        let runner = FakeRunner::default().with_output(
            "docker",
            &["--version"],
            EnvironmentCommandOutput::success("Docker version 26\n"),
        );

        let report = inspect_devcontainer_environment(
            temp.path(),
            &["cargo".to_string(), "check".to_string()],
            false,
            &runner,
        )
        .unwrap();

        assert_eq!(
            report.image_prepare.state,
            DevContainerPrepareState::ApprovalRequired
        );
        assert_eq!(report.healthcheck.state, DevContainerHealthState::NotReady);
        assert!(report.mount_plan.is_some());
    }

    #[test]
    fn devcontainer_runs_mocked_prepare_and_healthcheck() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join(".devcontainer")).unwrap();
        fs::write(
            temp.path().join(".devcontainer/devcontainer.json"),
            r#"{"name":"demo","image":"ubuntu:24.04"}"#,
        )
        .unwrap();
        let root = temp.path().to_string_lossy().to_string();
        let runner = FakeRunner::default()
            .with_output(
                "docker",
                &["--version"],
                EnvironmentCommandOutput::success("Docker version 26\n"),
            )
            .with_output(
                "devcontainer",
                &["up", "--workspace-folder", &root],
                EnvironmentCommandOutput::success("container ready\n"),
            )
            .with_output(
                "devcontainer",
                &["exec", "--workspace-folder", &root, "cargo", "check"],
                EnvironmentCommandOutput::success("checked\n"),
            );

        let report = inspect_devcontainer_environment(
            temp.path(),
            &["cargo".to_string(), "check".to_string()],
            true,
            &runner,
        )
        .unwrap();

        assert_eq!(report.engine, Some(ContainerEngine::Docker));
        assert_eq!(
            report.image_prepare.state,
            DevContainerPrepareState::Prepared
        );
        assert_eq!(
            report.healthcheck.state,
            DevContainerHealthState::ShellReady
        );
        assert!(report.healthcheck.shell_ready);
        assert_eq!(
            report.mount_plan.as_ref().unwrap().target,
            format!(
                "/workspaces/{}",
                temp.path().file_name().unwrap().to_string_lossy()
            )
        );
    }

    #[test]
    fn devcontainer_captures_prepare_failure_logs() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join(".devcontainer")).unwrap();
        fs::write(
            temp.path().join(".devcontainer/devcontainer.json"),
            r#"{"name":"demo","image":"ubuntu:24.04"}"#,
        )
        .unwrap();
        let root = temp.path().to_string_lossy().to_string();
        let runner = FakeRunner::default()
            .with_output(
                "docker",
                &["--version"],
                EnvironmentCommandOutput::success("Docker version 26\n"),
            )
            .with_output(
                "devcontainer",
                &["up", "--workspace-folder", &root],
                EnvironmentCommandOutput {
                    status_code: Some(1),
                    stdout: "pulling image\n".to_string(),
                    stderr: "pull denied\n".to_string(),
                },
            );

        let report = inspect_devcontainer_environment(temp.path(), &[], true, &runner).unwrap();

        assert_eq!(report.image_prepare.state, DevContainerPrepareState::Failed);
        assert!(report.image_prepare.logs.contains("pulling image"));
        assert!(report.image_prepare.logs.contains("pull denied"));
        assert_eq!(report.healthcheck.state, DevContainerHealthState::NotReady);
    }
}
