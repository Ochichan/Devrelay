//! Isolated task runner workspace preparation.
//!
//! M10 runner workspaces are disposable Git repositories under DevRelay's
//! project data directory. They apply immutable task execution snapshots without
//! taking writer ownership of the source workspace.

use crate::{
    CasStore, DevRelayError, DevRelayHome, EnvironmentKind, EnvironmentSelectionContext, GitRepo,
    Manifest, Result, SecretMaterializationReport, SecretProvider, SecretProviderLocalConfig,
    SnapshotStore, TaskExecutionSnapshot, VerificationDetails, apply_snapshot,
    apply_snapshot_with_sidecars, environment_profile_command_scope, materialize_project_secrets,
    profile_targets_platform,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const RUNNER_WORKSPACES_DIR: &str = "runner-workspaces";
const RUNNER_MARKER_FILE: &str = ".devrelay-runner-workspace";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRunnerWorkspaceOptions {
    pub task_run_id: String,
    pub environment_context: EnvironmentSelectionContext,
    pub retention_policy: TaskRunnerWorkspaceRetentionPolicy,
}

impl TaskRunnerWorkspaceOptions {
    pub fn new(
        task_run_id: impl Into<String>,
        environment_context: EnvironmentSelectionContext,
    ) -> Self {
        Self {
            task_run_id: task_run_id.into(),
            environment_context,
            retention_policy: TaskRunnerWorkspaceRetentionPolicy::delete_on_cleanup(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskRunnerWorkspaceRetentionPolicy {
    pub delete_on_cleanup: bool,
}

impl TaskRunnerWorkspaceRetentionPolicy {
    pub const fn delete_on_cleanup() -> Self {
        Self {
            delete_on_cleanup: true,
        }
    }

    pub const fn keep_for_debug() -> Self {
        Self {
            delete_on_cleanup: false,
        }
    }
}

pub enum TaskRunnerSecretPolicy<'a> {
    NotPermitted,
    Permitted {
        local_config: &'a SecretProviderLocalConfig,
        provider: &'a dyn SecretProvider,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskRunnerWorkspace {
    pub task_run_id: String,
    pub project_id: String,
    pub task_name: String,
    pub path: PathBuf,
    pub snapshot_id: String,
    pub canonical_session: bool,
    pub environment: TaskRunnerEnvironmentState,
    pub sidecars: TaskRunnerSidecarState,
    pub secrets: TaskRunnerSecretState,
    pub verification: VerificationDetails,
    pub retention_policy: TaskRunnerWorkspaceRetentionPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskRunnerEnvironmentState {
    pub profile_name: String,
    pub kind: EnvironmentKind,
    pub command_scope: String,
    pub hydrated: bool,
    pub explanation: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum TaskRunnerSidecarState {
    NotRequired,
    Materialized { count: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum TaskRunnerSecretState {
    SkippedNotPermitted { required: Vec<String> },
    Materialized { report: SecretMaterializationReport },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskRunnerWorkspaceCleanup {
    pub path: PathBuf,
    pub removed: bool,
}

pub fn prepare_task_runner_workspace(
    home: &DevRelayHome,
    manifest: &Manifest,
    store: &SnapshotStore,
    execution: &TaskExecutionSnapshot,
    options: TaskRunnerWorkspaceOptions,
    cas_store: Option<&CasStore>,
    secret_policy: TaskRunnerSecretPolicy<'_>,
) -> Result<TaskRunnerWorkspace> {
    if execution.definition.project_id != manifest.project_id {
        return Err(DevRelayError::Config(format!(
            "task execution project_id {} does not match manifest project_id {}",
            execution.definition.project_id, manifest.project_id
        )));
    }
    validate_task_run_id(&options.task_run_id)?;
    let environment = hydrate_task_environment(manifest, execution, &options.environment_context)?;
    let workspace_path =
        task_runner_workspace_path(home, &manifest.project_id, &options.task_run_id);
    create_isolated_git_workspace(&workspace_path)?;

    let target = GitRepo::new(&workspace_path);
    let source = GitRepo::new(store.snapshot_repo_path());
    let sidecars = materialize_runner_sidecars(&target, &source, execution, cas_store)?;
    let secrets = materialize_runner_secrets(&workspace_path, manifest, secret_policy)?;

    Ok(TaskRunnerWorkspace {
        task_run_id: options.task_run_id,
        project_id: manifest.project_id.clone(),
        task_name: execution.definition.task_name.clone(),
        path: workspace_path,
        snapshot_id: execution.snapshot.snapshot_id.clone(),
        canonical_session: false,
        environment,
        sidecars: sidecars.0,
        secrets,
        verification: sidecars.1,
        retention_policy: options.retention_policy,
    })
}

pub fn cleanup_task_runner_workspace(
    workspace: &TaskRunnerWorkspace,
) -> Result<TaskRunnerWorkspaceCleanup> {
    if !workspace.retention_policy.delete_on_cleanup {
        return Ok(TaskRunnerWorkspaceCleanup {
            path: workspace.path.clone(),
            removed: false,
        });
    }
    if workspace.path.exists() {
        fs::remove_dir_all(&workspace.path)?;
    }
    Ok(TaskRunnerWorkspaceCleanup {
        path: workspace.path.clone(),
        removed: true,
    })
}

pub fn task_runner_workspace_path(
    home: &DevRelayHome,
    project_id: &str,
    task_run_id: &str,
) -> PathBuf {
    home.project_data_dir(project_id)
        .join(RUNNER_WORKSPACES_DIR)
        .join(task_run_id)
}

fn hydrate_task_environment(
    manifest: &Manifest,
    execution: &TaskExecutionSnapshot,
    context: &EnvironmentSelectionContext,
) -> Result<TaskRunnerEnvironmentState> {
    let profile_name = &execution.definition.profile_name;
    let profile = manifest
        .environment
        .as_ref()
        .and_then(|environment| environment.profiles.get(profile_name))
        .ok_or_else(|| {
            DevRelayError::Config(format!(
                "task profile {profile_name:?} is missing from manifest environment"
            ))
        })?;
    if !profile_targets_platform(&profile.targets, &context.platform_key) {
        return Err(DevRelayError::Config(format!(
            "task profile {profile_name:?} does not target {}",
            context.platform_key
        )));
    }
    if !context.available_kinds.contains(&profile.kind) {
        return Err(DevRelayError::Config(format!(
            "task profile {profile_name:?} requires {:?} environment support",
            profile.kind
        )));
    }
    let command_scope = environment_profile_command_scope(profile_name);
    if profile.kind == EnvironmentKind::Script
        && !context.trusted_command_scopes.contains(&command_scope)
    {
        return Err(DevRelayError::Config(format!(
            "task profile {profile_name:?} script command is not trusted"
        )));
    }

    Ok(TaskRunnerEnvironmentState {
        profile_name: profile_name.clone(),
        kind: profile.kind,
        command_scope,
        hydrated: true,
        explanation: vec![format!(
            "prepared {:?} profile {profile_name} for {}",
            profile.kind, context.platform_key
        )],
    })
}

fn materialize_runner_sidecars(
    target: &GitRepo,
    source: &GitRepo,
    execution: &TaskExecutionSnapshot,
    cas_store: Option<&CasStore>,
) -> Result<(TaskRunnerSidecarState, VerificationDetails)> {
    if execution.snapshot.metadata.sidecars.is_empty() {
        return apply_snapshot(target, source, &execution.snapshot.metadata)
            .map(|verification| (TaskRunnerSidecarState::NotRequired, verification));
    }
    let Some(cas_store) = cas_store else {
        return Err(DevRelayError::Config(format!(
            "task snapshot {} requires sidecar materialization but no CAS store was provided",
            execution.snapshot.snapshot_id
        )));
    };
    let count = execution.snapshot.metadata.sidecars.len();
    let verification =
        apply_snapshot_with_sidecars(target, source, &execution.snapshot.metadata, cas_store)?;
    Ok((TaskRunnerSidecarState::Materialized { count }, verification))
}

fn materialize_runner_secrets(
    workspace_path: &Path,
    manifest: &Manifest,
    secret_policy: TaskRunnerSecretPolicy<'_>,
) -> Result<TaskRunnerSecretState> {
    match secret_policy {
        TaskRunnerSecretPolicy::NotPermitted => Ok(TaskRunnerSecretState::SkippedNotPermitted {
            required: required_secret_names(manifest),
        }),
        TaskRunnerSecretPolicy::Permitted {
            local_config,
            provider,
        } => materialize_project_secrets(workspace_path, manifest, local_config, provider)
            .map(|report| TaskRunnerSecretState::Materialized { report }),
    }
}

fn required_secret_names(manifest: &Manifest) -> Vec<String> {
    manifest
        .secrets
        .iter()
        .filter(|(_, secret)| secret.required)
        .map(|(name, _)| name.clone())
        .collect()
}

fn create_isolated_git_workspace(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(path)?;
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["init", "-b", "runner"])
        .output()?;
    if !output.status.success() {
        return Err(DevRelayError::GitCommand {
            cwd: path.to_path_buf(),
            args: "init -b runner".to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    fs::write(
        path.join(".git").join(RUNNER_MARKER_FILE),
        "devrelay task runner workspace\n",
    )?;
    Ok(())
}

fn validate_task_run_id(task_run_id: &str) -> Result<()> {
    if task_run_id.is_empty()
        || task_run_id.len() > 128
        || task_run_id.bytes().any(
            |byte| !matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.'),
        )
    {
        return Err(DevRelayError::Config(format!(
            "invalid task run id {task_run_id:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        SecretProviderKind, SecretProviderMapping, SecretProviderRequest, SecretValue,
        TaskRunInput, TaskRunState, create_task_execution_snapshot,
    };
    use std::collections::BTreeMap;
    use std::process::Command;

    #[test]
    fn prepares_isolated_runner_workspace_from_execution_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        fs::write(source_path.join("tracked.txt"), "runner content\n").unwrap();
        let manifest = manifest(false);
        let mut store = SnapshotStore::open(&home, &manifest.project_id).unwrap();
        let execution = create_task_execution_snapshot(
            &source,
            &manifest,
            &mut store,
            "test",
            Some("se_runner".to_string()),
        )
        .unwrap();

        let workspace = prepare_task_runner_workspace(
            &home,
            &manifest,
            &store,
            &execution,
            options("tr_workspace"),
            None,
            TaskRunnerSecretPolicy::NotPermitted,
        )
        .unwrap();

        assert!(
            workspace
                .path
                .starts_with(home.project_data_dir(&manifest.project_id))
        );
        assert!(workspace.path.ends_with("tr_workspace"));
        assert_ne!(workspace.path, source_path);
        assert!(!workspace.canonical_session);
        assert_eq!(workspace.snapshot_id, execution.snapshot.snapshot_id);
        assert_eq!(workspace.environment.profile_name, "dev");
        assert_eq!(workspace.sidecars, TaskRunnerSidecarState::NotRequired);
        assert_eq!(
            read_text_lf(workspace.path.join("tracked.txt")),
            "runner content\n"
        );
        assert!(
            workspace
                .path
                .join(".git")
                .join(RUNNER_MARKER_FILE)
                .exists()
        );

        let cleanup = cleanup_task_runner_workspace(&workspace).unwrap();
        assert!(cleanup.removed);
        assert!(!workspace.path.exists());
    }

    #[test]
    fn skips_required_secrets_when_not_permitted() {
        let (home, manifest, store, execution) = snapshot_fixture(true);

        let workspace = prepare_task_runner_workspace(
            &home,
            &manifest,
            &store,
            &execution,
            options("tr_no_secrets"),
            None,
            TaskRunnerSecretPolicy::NotPermitted,
        )
        .unwrap();

        assert_eq!(
            workspace.secrets,
            TaskRunnerSecretState::SkippedNotPermitted {
                required: vec!["api_key".to_string()],
            }
        );
        assert!(!workspace.path.join(".secrets/api-key").exists());
    }

    #[test]
    fn materializes_required_secrets_when_permitted() {
        let (home, manifest, store, execution) = snapshot_fixture(true);
        let mut local_config = SecretProviderLocalConfig::default();
        local_config.mappings.insert(
            "api_key".to_string(),
            SecretProviderMapping {
                provider: SecretProviderKind::OsKeychain,
                reference: "api-key".to_string(),
                command: Vec::new(),
            },
        );
        let provider = MapSecretProvider(BTreeMap::from([(
            "api_key".to_string(),
            "secret-value".to_string(),
        )]));

        let workspace = prepare_task_runner_workspace(
            &home,
            &manifest,
            &store,
            &execution,
            options("tr_with_secrets"),
            None,
            TaskRunnerSecretPolicy::Permitted {
                local_config: &local_config,
                provider: &provider,
            },
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(workspace.path.join(".secrets/api-key")).unwrap(),
            "secret-value"
        );
        match workspace.secrets {
            TaskRunnerSecretState::Materialized { report } => {
                assert_eq!(report.files.len(), 1);
                assert_eq!(report.hard_exclude_patterns, vec![".secrets/api-key"]);
            }
            other => panic!("unexpected secret state: {other:?}"),
        }
    }

    #[test]
    fn rejects_runner_workspace_as_existing_path() {
        let (home, manifest, store, execution) = snapshot_fixture(false);
        let path = task_runner_workspace_path(&home, &manifest.project_id, "tr_existing");
        fs::create_dir_all(&path).unwrap();

        let err = prepare_task_runner_workspace(
            &home,
            &manifest,
            &store,
            &execution,
            options("tr_existing"),
            None,
            TaskRunnerSecretPolicy::NotPermitted,
        )
        .unwrap_err();

        assert!(err.to_string().contains("File exists") || err.to_string().contains("exists"));
    }

    fn snapshot_fixture(
        with_secret: bool,
    ) -> (DevRelayHome, Manifest, SnapshotStore, TaskExecutionSnapshot) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.keep();
        let home = DevRelayHome::new(root.join("home"));
        let source_path = root.join("source");
        let source = init_repo(&source_path);
        fs::write(source_path.join("tracked.txt"), "runner content\n").unwrap();
        let manifest = manifest(with_secret);
        let mut store = SnapshotStore::open(&home, &manifest.project_id).unwrap();
        let task_run = TaskRunInput::new(
            manifest.project_id.clone(),
            TaskRunState::Queued,
            serde_json::json!({ "test": true }),
        );
        let execution = create_task_execution_snapshot(
            &source,
            &manifest,
            &mut store,
            "test",
            Some(task_run.task_run_id),
        )
        .unwrap();
        (home, manifest, store, execution)
    }

    fn manifest(with_secret: bool) -> Manifest {
        let secret = if with_secret {
            r#"
[secrets.api_key]
target = ".secrets/api-key"
required = true
"#
        } else {
            ""
        };
        Manifest::parse(&format!(
            r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"

[environment.profiles.dev]
kind = "native"
targets = ["*"]
command = ["bash", "-lc", "true"]
healthcheck = ["bash", "-lc", "true"]

[tasks.test]
profile = "dev"
command = ["bash", "-lc", "cat tracked.txt"]
platforms = ["*"]
cpu = 1
memory_mib = 64
disk_mib = 64
{secret}
"#
        ))
        .unwrap()
    }

    fn options(task_run_id: &str) -> TaskRunnerWorkspaceOptions {
        TaskRunnerWorkspaceOptions::new(
            task_run_id,
            EnvironmentSelectionContext::with_platform_key(crate::current_platform_key())
                .with_available_kind(EnvironmentKind::Native),
        )
    }

    fn read_text_lf(path: impl AsRef<Path>) -> String {
        fs::read_to_string(path).unwrap().replace("\r\n", "\n")
    }

    fn init_repo(path: &Path) -> GitRepo {
        fs::create_dir_all(path).unwrap();
        run_git(path, &["init", "-b", "main"]);
        run_git(path, &["config", "core.autocrlf", "false"]);
        run_git(path, &["config", "core.eol", "lf"]);
        run_git(path, &["config", "user.name", "DevRelay Test"]);
        run_git(
            path,
            &["config", "user.email", "devrelay-test@example.local"],
        );
        fs::write(path.join("tracked.txt"), "base\n").unwrap();
        run_git(path, &["add", "tracked.txt"]);
        run_git(path, &["commit", "-m", "base"]);
        GitRepo::new(path)
    }

    fn run_git(path: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git command failed: {args:?}");
    }

    struct MapSecretProvider(BTreeMap<String, String>);

    impl SecretProvider for MapSecretProvider {
        fn resolve_secret(&self, request: &SecretProviderRequest) -> Result<Option<SecretValue>> {
            Ok(self
                .0
                .get(&request.secret_name)
                .map(|value| SecretValue::new(value.clone())))
        }
    }
}
