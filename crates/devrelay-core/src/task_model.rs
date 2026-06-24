//! Task definition normalization and run metadata helpers.
//!
//! M10 task execution starts from an immutable definition: the task command,
//! selected environment profile, constraints, and resource hints are reduced to
//! a stable hash before any scheduler or remote runner chooses a device.

use crate::{
    DevRelayError, EnvironmentKind, EnvironmentProfile, GitRepo, Manifest, Result, SnapshotStore,
    StoredSnapshot, TaskCacheMode, TaskConfig, TaskSandbox, create_snapshot,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub const TASK_RUN_ID_PREFIX: &str = "tr_";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskDefinition {
    pub project_id: String,
    pub task_name: String,
    pub profile_name: String,
    pub profile_kind: EnvironmentKind,
    pub command: Vec<String>,
    pub platforms: Vec<String>,
    pub cpu: Option<u64>,
    pub memory_mib: Option<u64>,
    pub disk_mib: Option<u64>,
    pub interactive: bool,
    pub cache: Option<TaskCacheMode>,
    pub outputs: Vec<String>,
    pub features: Vec<String>,
    pub sandbox: Option<TaskSandbox>,
    pub command_definition_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskExecutionSnapshot {
    pub definition: TaskDefinition,
    pub snapshot: StoredSnapshot,
    pub label: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TaskRunState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Canceled,
}

impl TaskRunState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "canceled" => Self::Canceled,
            _ => Self::Queued,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskRunInput {
    pub task_run_id: String,
    pub project_id: String,
    pub session_id: Option<String>,
    pub state: TaskRunState,
    pub command: Option<String>,
    pub metadata: serde_json::Value,
}

impl TaskRunInput {
    pub fn new(
        project_id: impl Into<String>,
        state: TaskRunState,
        metadata: serde_json::Value,
    ) -> Self {
        Self {
            task_run_id: generate_task_run_id(),
            project_id: project_id.into(),
            session_id: None,
            state,
            command: None,
            metadata,
        }
    }
}

pub fn task_definitions_from_manifest(
    manifest: &Manifest,
) -> Result<BTreeMap<String, TaskDefinition>> {
    manifest
        .tasks
        .keys()
        .map(|name| Ok((name.clone(), task_definition(manifest, name)?)))
        .collect()
}

pub fn task_definition(manifest: &Manifest, task_name: &str) -> Result<TaskDefinition> {
    let task = manifest
        .tasks
        .get(task_name)
        .ok_or_else(|| DevRelayError::Config(format!("unknown task definition {task_name:?}")))?;
    let profile = profile_for_task(manifest, task_name, task)?;
    let command_definition_hash =
        task_command_definition_hash_parts(&manifest.project_id, task_name, task, profile);
    Ok(TaskDefinition {
        project_id: manifest.project_id.clone(),
        task_name: task_name.to_string(),
        profile_name: task.profile.clone(),
        profile_kind: profile.kind,
        command: task.command.clone(),
        platforms: sorted(task.platforms.clone()),
        cpu: task.cpu,
        memory_mib: task.memory_mib,
        disk_mib: task.disk_mib,
        interactive: task.interactive,
        cache: task.cache,
        outputs: sorted(task.outputs.clone()),
        features: sorted(task.features.clone()),
        sandbox: task.sandbox,
        command_definition_hash,
    })
}

pub fn task_command_definition_hash(manifest: &Manifest, task_name: &str) -> Result<String> {
    let task = manifest
        .tasks
        .get(task_name)
        .ok_or_else(|| DevRelayError::Config(format!("unknown task definition {task_name:?}")))?;
    let profile = profile_for_task(manifest, task_name, task)?;
    Ok(task_command_definition_hash_parts(
        &manifest.project_id,
        task_name,
        task,
        profile,
    ))
}

pub fn create_task_execution_snapshot(
    source: &GitRepo,
    manifest: &Manifest,
    store: &mut SnapshotStore,
    task_name: &str,
    session_id: Option<String>,
) -> Result<TaskExecutionSnapshot> {
    let definition = task_definition(manifest, task_name)?;
    let mut metadata = create_snapshot(source, manifest)?;
    metadata.session_id = session_id;
    metadata.validate()?;
    let label = task_execution_snapshot_label(&definition);
    let snapshot = store.store_snapshot(source, metadata, true, Some(label.clone()))?;
    Ok(TaskExecutionSnapshot {
        definition,
        snapshot,
        label,
    })
}

pub fn task_execution_snapshot_label(definition: &TaskDefinition) -> String {
    let hash_prefix = definition
        .command_definition_hash
        .get(..12)
        .unwrap_or(&definition.command_definition_hash);
    format!("task:{}:{hash_prefix}", definition.task_name)
}

pub fn generate_task_run_id() -> String {
    let seed = format!("{}\0{}", std::process::id(), unix_now_nanos());
    let digest = blake3::hash(seed.as_bytes());
    format!("{TASK_RUN_ID_PREFIX}{}", &digest.to_hex()[..24])
}

fn profile_for_task<'a>(
    manifest: &'a Manifest,
    task_name: &str,
    task: &TaskConfig,
) -> Result<&'a EnvironmentProfile> {
    manifest
        .environment
        .as_ref()
        .and_then(|environment| environment.profiles.get(&task.profile))
        .ok_or_else(|| {
            DevRelayError::Config(format!(
                "task {task_name:?} references unknown environment profile {:?}",
                task.profile
            ))
        })
}

fn task_command_definition_hash_parts(
    project_id: &str,
    task_name: &str,
    task: &TaskConfig,
    profile: &EnvironmentProfile,
) -> String {
    let mut hasher = blake3::Hasher::new();
    update_hash_field(&mut hasher, "devrelay.task-command.v1");
    update_hash_field(&mut hasher, project_id);
    update_hash_field(&mut hasher, task_name);
    update_hash_field(&mut hasher, &task.profile);
    update_hash_command(&mut hasher, "task.command", &task.command);
    update_hash_sorted_values(&mut hasher, "task.platforms", &task.platforms);
    update_hash_optional_u64(&mut hasher, "task.cpu", task.cpu);
    update_hash_optional_u64(&mut hasher, "task.memory_mib", task.memory_mib);
    update_hash_optional_u64(&mut hasher, "task.disk_mib", task.disk_mib);
    update_hash_field(&mut hasher, "task.interactive");
    update_hash_field(&mut hasher, if task.interactive { "true" } else { "false" });
    update_hash_optional_debug(&mut hasher, "task.cache", task.cache);
    update_hash_sorted_values(&mut hasher, "task.outputs", &task.outputs);
    update_hash_sorted_values(&mut hasher, "task.features", &task.features);
    update_hash_optional_debug(&mut hasher, "task.sandbox", task.sandbox);

    update_hash_field(&mut hasher, "profile.kind");
    update_hash_field(&mut hasher, &format!("{:?}", profile.kind));
    update_hash_sorted_values(&mut hasher, "profile.targets", &profile.targets);
    update_hash_command(&mut hasher, "profile.command", &profile.command);
    update_hash_sorted_values(
        &mut hasher,
        "profile.fingerprint_files",
        &profile.fingerprint_files,
    );
    if let Some(healthcheck) = &profile.healthcheck {
        update_hash_command(&mut hasher, "profile.healthcheck", healthcheck);
    } else {
        update_hash_field(&mut hasher, "profile.healthcheck.none");
    }
    update_hash_optional_string(
        &mut hasher,
        "profile.working_directory",
        &profile.working_directory,
    );
    update_hash_optional_u64(
        &mut hasher,
        "profile.timeout_seconds",
        profile.timeout_seconds,
    );
    hasher.finalize().to_hex().to_string()
}

fn sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}

fn update_hash_command(hasher: &mut blake3::Hasher, field: &str, values: &[String]) {
    update_hash_field(hasher, field);
    for value in values {
        update_hash_field(hasher, value);
    }
}

fn update_hash_sorted_values(hasher: &mut blake3::Hasher, field: &str, values: &[String]) {
    update_hash_field(hasher, field);
    for value in sorted(values.to_vec()) {
        update_hash_field(hasher, &value);
    }
}

fn update_hash_optional_u64(hasher: &mut blake3::Hasher, field: &str, value: Option<u64>) {
    update_hash_field(hasher, field);
    match value {
        Some(value) => update_hash_field(hasher, &value.to_string()),
        None => update_hash_field(hasher, "none"),
    }
}

fn update_hash_optional_debug<T: std::fmt::Debug>(
    hasher: &mut blake3::Hasher,
    field: &str,
    value: Option<T>,
) {
    update_hash_field(hasher, field);
    match value {
        Some(value) => update_hash_field(hasher, &format!("{value:?}")),
        None => update_hash_field(hasher, "none"),
    }
}

fn update_hash_optional_string(hasher: &mut blake3::Hasher, field: &str, value: &Option<String>) {
    update_hash_field(hasher, field);
    match value {
        Some(value) => update_hash_field(hasher, value),
        None => update_hash_field(hasher, "none"),
    }
}

fn update_hash_field(hasher: &mut blake3::Hasher, value: &str) {
    hasher.update(value.as_bytes());
    hasher.update(&[0]);
}

fn unix_now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DevRelayHome;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    fn manifest(raw: &str) -> Manifest {
        Manifest::parse(raw).unwrap()
    }

    fn task_manifest(command: &str) -> Manifest {
        manifest(&format!(
            r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"

[environment.profiles.dev]
kind = "native"
targets = ["darwin", "linux"]
command = ["bash", "-lc", "setup"]
healthcheck = ["bash", "-lc", "true"]

[tasks.test]
profile = "dev"
command = {command}
platforms = ["linux-*", "darwin-*"]
cpu = 4
memory_mib = 4096
disk_mib = 0
interactive = false
cache = "read-write"
outputs = ["test-results/**"]
features = ["python"]
sandbox = "container"
"#
        ))
    }

    #[test]
    fn normalizes_task_definition_from_manifest() {
        let manifest = task_manifest(r#"["bash", "-lc", "pytest -q"]"#);
        let definition = task_definition(&manifest, "test").unwrap();

        assert_eq!(definition.project_id, "12345678");
        assert_eq!(definition.task_name, "test");
        assert_eq!(definition.profile_name, "dev");
        assert_eq!(definition.profile_kind, EnvironmentKind::Native);
        assert_eq!(definition.command, vec!["bash", "-lc", "pytest -q"]);
        assert_eq!(definition.cpu, Some(4));
        assert_eq!(definition.memory_mib, Some(4096));
        assert_eq!(definition.cache, Some(TaskCacheMode::ReadWrite));
        assert_eq!(definition.sandbox, Some(TaskSandbox::Container));
        assert_eq!(definition.command_definition_hash.len(), 64);

        let definitions = task_definitions_from_manifest(&manifest).unwrap();
        assert_eq!(definitions.get("test"), Some(&definition));
    }

    #[test]
    fn task_command_definition_hash_changes_for_command_and_profile_edits() {
        let original = task_manifest(r#"["bash", "-lc", "pytest -q"]"#);
        let changed_task = task_manifest(r#"["bash", "-lc", "pytest -q tests/unit"]"#);
        assert_ne!(
            task_command_definition_hash(&original, "test").unwrap(),
            task_command_definition_hash(&changed_task, "test").unwrap()
        );

        let mut changed_profile = original.clone();
        let profile = changed_profile
            .environment
            .as_mut()
            .unwrap()
            .profiles
            .get_mut("dev")
            .unwrap();
        profile.command = vec![
            "bash".to_string(),
            "-lc".to_string(),
            "setup v2".to_string(),
        ];
        assert_ne!(
            task_command_definition_hash(&original, "test").unwrap(),
            task_command_definition_hash(&changed_profile, "test").unwrap()
        );
    }

    #[test]
    fn creates_pinned_task_execution_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let manifest = task_manifest(r#"["bash", "-lc", "pytest -q"]"#);
        let mut store = SnapshotStore::open(&home, &manifest.project_id).unwrap();

        let execution = create_task_execution_snapshot(
            &source,
            &manifest,
            &mut store,
            "test",
            Some("se_task".to_string()),
        )
        .unwrap();

        assert!(execution.snapshot.pinned);
        assert_eq!(execution.snapshot.session_id.as_deref(), Some("se_task"));
        assert_eq!(
            execution.snapshot.label.as_deref(),
            Some(execution.label.as_str())
        );
        assert_eq!(
            execution.label,
            task_execution_snapshot_label(&execution.definition)
        );
        assert!(execution.label.contains("test"));
        assert_eq!(
            execution.definition.command_definition_hash,
            task_command_definition_hash(&manifest, "test").unwrap()
        );
        assert!(
            source
                .run(&[
                    "rev-parse",
                    "--verify",
                    &execution.snapshot.metadata.index_ref()
                ])
                .is_err()
        );
        assert!(
            GitRepo::new(store.snapshot_repo_path())
                .run(&[
                    "rev-parse",
                    "--verify",
                    &execution.snapshot.metadata.index_ref()
                ])
                .is_ok()
        );
        assert_eq!(
            store
                .list_snapshots()
                .unwrap()
                .into_iter()
                .map(|snapshot| snapshot.snapshot_id)
                .collect::<Vec<_>>(),
            vec![execution.snapshot.snapshot_id]
        );
    }

    #[test]
    fn task_run_ids_use_stable_prefix() {
        let id = generate_task_run_id();
        assert!(id.starts_with(TASK_RUN_ID_PREFIX));
        assert_eq!(id.len(), TASK_RUN_ID_PREFIX.len() + 24);
    }

    fn init_repo(path: &Path) -> GitRepo {
        fs::create_dir_all(path).unwrap();
        run_git(path, &["init", "-b", "main"]);
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
}
