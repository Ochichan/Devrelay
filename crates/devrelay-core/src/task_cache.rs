//! Task result cache keying, metadata, and artifact restore helpers.
//!
//! The cache key is derived only from deterministic task inputs. Secret values
//! are never accepted by this module, and manifests that declare secrets are
//! treated as cache-disabled unless the caller explicitly overrides the policy.

use crate::{
    CasStore, DevRelayError, DevRelayHome, Manifest, Result, TaskArtifactIndex,
    TaskArtifactPullResult, TaskCacheMode, TaskDefinition, TaskExecutionSnapshot,
    pull_task_artifact,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const TASK_RESULT_CACHE_DIR: &str = "task-result-cache";
const TASK_RESULT_CACHE_KEY_PREFIX: &str = "tc_";
const TASK_RESULT_CACHE_KEY_HEX_LEN: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskResultCacheKey {
    pub key: String,
    pub parts: TaskResultCacheKeyParts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskResultCacheKeyParts {
    pub schema: String,
    pub project_id: String,
    pub task_name: String,
    pub command_definition_hash: String,
    pub platform_key: String,
    pub environment_fingerprint: String,
    pub input_state_hash: String,
    pub input_head_oid: String,
    pub input_index_tree_oid: String,
    pub input_work_tree_oid: String,
    pub sidecars: Vec<TaskResultCacheSidecarInput>,
    pub outputs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskResultCacheSidecarInput {
    pub logical_path: String,
    pub file_mode: String,
    pub classification: String,
    pub size_bytes: u64,
    pub chunk_size_bytes: u64,
    pub root_hash: String,
    pub cas_manifest_id: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskResultCachePolicy {
    pub allow_secret_sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskResultCacheEligibility {
    pub read: bool,
    pub write: bool,
    pub secret_sensitive: bool,
    pub explanation: String,
}

impl TaskResultCacheEligibility {
    pub fn enabled(&self) -> bool {
        self.read || self.write
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskResultCacheEntry {
    pub key: String,
    pub key_parts: TaskResultCacheKeyParts,
    pub project_id: String,
    pub task_name: String,
    pub task_run_id: String,
    pub artifact_index: TaskArtifactIndex,
    pub created_at_unix_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskResultCacheStoreResult {
    pub path: PathBuf,
    pub entry: TaskResultCacheEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskResultCacheHit {
    pub path: PathBuf,
    pub entry: TaskResultCacheEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskResultCacheRestore {
    pub restored: Vec<TaskArtifactPullResult>,
    pub total_bytes: u64,
}

pub fn task_result_cache_key(
    execution: &TaskExecutionSnapshot,
    platform_key: &str,
    environment_fingerprint: &str,
) -> Result<TaskResultCacheKey> {
    validate_non_empty("platform_key", platform_key)?;
    validate_non_empty("environment_fingerprint", environment_fingerprint)?;
    execution.snapshot.metadata.validate()?;

    let definition = &execution.definition;
    let metadata = &execution.snapshot.metadata;
    let mut sidecars = metadata
        .sidecars
        .iter()
        .map(|sidecar| TaskResultCacheSidecarInput {
            logical_path: sidecar.logical_path.clone(),
            file_mode: sidecar.file_mode.clone(),
            classification: sidecar.classification.clone(),
            size_bytes: sidecar.size_bytes,
            chunk_size_bytes: sidecar.chunk_size_bytes,
            root_hash: sidecar.root_hash.clone(),
            cas_manifest_id: sidecar.cas_manifest_id.clone(),
        })
        .collect::<Vec<_>>();
    sidecars.sort_by(|left, right| {
        left.logical_path
            .cmp(&right.logical_path)
            .then(left.cas_manifest_id.cmp(&right.cas_manifest_id))
    });

    let parts = TaskResultCacheKeyParts {
        schema: "devrelay.task-result-cache.v1".to_string(),
        project_id: definition.project_id.clone(),
        task_name: definition.task_name.clone(),
        command_definition_hash: definition.command_definition_hash.clone(),
        platform_key: platform_key.to_string(),
        environment_fingerprint: environment_fingerprint.to_string(),
        input_state_hash: metadata.state_hash.clone(),
        input_head_oid: metadata.head_oid.clone(),
        input_index_tree_oid: metadata.index_tree_oid.clone(),
        input_work_tree_oid: metadata.work_tree_oid.clone(),
        sidecars,
        outputs: definition.outputs.clone(),
    };
    let key = hash_cache_key_parts(&parts)?;
    Ok(TaskResultCacheKey { key, parts })
}

pub fn task_result_cache_eligibility(
    definition: &TaskDefinition,
    manifest: &Manifest,
    policy: TaskResultCachePolicy,
) -> TaskResultCacheEligibility {
    let (read, write, mode_explanation) = match definition.cache.unwrap_or(TaskCacheMode::Off) {
        TaskCacheMode::Off => (false, false, "task cache is off"),
        TaskCacheMode::Read => (true, false, "task cache is read-only"),
        TaskCacheMode::Write => (false, true, "task cache is write-only"),
        TaskCacheMode::ReadWrite => (true, true, "task cache is read-write"),
    };
    let secret_sensitive = !manifest.secrets.is_empty();
    if secret_sensitive && !policy.allow_secret_sensitive {
        return TaskResultCacheEligibility {
            read: false,
            write: false,
            secret_sensitive,
            explanation: "task cache disabled because the manifest declares secrets".to_string(),
        };
    }
    TaskResultCacheEligibility {
        read,
        write,
        secret_sensitive,
        explanation: mode_explanation.to_string(),
    }
}

pub fn store_task_result_cache(
    home: &DevRelayHome,
    key: &TaskResultCacheKey,
    artifact_index: &TaskArtifactIndex,
    eligibility: &TaskResultCacheEligibility,
) -> Result<Option<TaskResultCacheStoreResult>> {
    if !eligibility.write {
        return Ok(None);
    }
    validate_cache_result_matches_key(key, artifact_index)?;
    let path = task_result_cache_entry_path(home, &key.parts.project_id, &key.key)?;
    let entry = TaskResultCacheEntry {
        key: key.key.clone(),
        key_parts: key.parts.clone(),
        project_id: key.parts.project_id.clone(),
        task_name: key.parts.task_name.clone(),
        task_run_id: artifact_index.task_run_id.clone(),
        artifact_index: artifact_index.clone(),
        created_at_unix_seconds: unix_now_seconds(),
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_vec_pretty(&entry)?)?;
    Ok(Some(TaskResultCacheStoreResult { path, entry }))
}

pub fn lookup_task_result_cache(
    home: &DevRelayHome,
    key: &TaskResultCacheKey,
    eligibility: &TaskResultCacheEligibility,
) -> Result<Option<TaskResultCacheHit>> {
    if !eligibility.read {
        return Ok(None);
    }
    let path = task_result_cache_entry_path(home, &key.parts.project_id, &key.key)?;
    let entry = match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str::<TaskResultCacheEntry>(&raw)?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    if entry.key != key.key || entry.key_parts != key.parts {
        return Err(DevRelayError::Config(format!(
            "task result cache entry {} does not match requested key",
            key.key
        )));
    }
    validate_cache_result_matches_key(key, &entry.artifact_index)?;
    Ok(Some(TaskResultCacheHit { path, entry }))
}

pub fn restore_task_result_cache_hit(
    hit: &TaskResultCacheHit,
    cas_store: &CasStore,
    destination_root: impl AsRef<Path>,
) -> Result<TaskResultCacheRestore> {
    let destination_root = destination_root.as_ref();
    let mut restored = Vec::new();
    let mut total_bytes = 0_u64;
    for artifact in &hit.entry.artifact_index.entries {
        let pull = pull_task_artifact(
            &hit.entry.artifact_index,
            &artifact.path,
            cas_store,
            destination_root,
        )?;
        total_bytes = total_bytes.saturating_add(pull.size_bytes);
        restored.push(pull);
    }
    Ok(TaskResultCacheRestore {
        restored,
        total_bytes,
    })
}

pub fn read_task_result_cache_entry(path: impl AsRef<Path>) -> Result<TaskResultCacheEntry> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

pub fn task_result_cache_entry_path(
    home: &DevRelayHome,
    project_id: &str,
    key: &str,
) -> Result<PathBuf> {
    validate_task_result_cache_key(key)?;
    Ok(home
        .project_data_dir(project_id)
        .join(TASK_RESULT_CACHE_DIR)
        .join(format!("{key}.json")))
}

fn validate_cache_result_matches_key(
    key: &TaskResultCacheKey,
    artifact_index: &TaskArtifactIndex,
) -> Result<()> {
    for (field, actual, expected) in [
        (
            "project_id",
            artifact_index.project_id.as_str(),
            key.parts.project_id.as_str(),
        ),
        (
            "task_name",
            artifact_index.task_name.as_str(),
            key.parts.task_name.as_str(),
        ),
        (
            "platform_key",
            artifact_index.platform_key.as_str(),
            key.parts.platform_key.as_str(),
        ),
        (
            "command_definition_hash",
            artifact_index.command_definition_hash.as_str(),
            key.parts.command_definition_hash.as_str(),
        ),
    ] {
        if actual != expected {
            return Err(DevRelayError::Config(format!(
                "task result cache artifact index {field} {actual:?} does not match key {expected:?}"
            )));
        }
    }
    Ok(())
}

fn hash_cache_key_parts(parts: &TaskResultCacheKeyParts) -> Result<String> {
    let encoded = serde_json::to_vec(parts)?;
    let digest = blake3::hash(&encoded);
    let hex = digest.to_hex();
    Ok(format!(
        "{TASK_RESULT_CACHE_KEY_PREFIX}{}",
        &hex[..TASK_RESULT_CACHE_KEY_HEX_LEN]
    ))
}

fn validate_task_result_cache_key(key: &str) -> Result<()> {
    let Some(hex) = key.strip_prefix(TASK_RESULT_CACHE_KEY_PREFIX) else {
        return Err(DevRelayError::Config(format!(
            "invalid task result cache key {key:?}"
        )));
    };
    if hex.len() != TASK_RESULT_CACHE_KEY_HEX_LEN
        || !hex.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DevRelayError::Config(format!(
            "invalid task result cache key {key:?}"
        )));
    }
    Ok(())
}

fn validate_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(DevRelayError::Config(format!(
            "task result cache {field} must not be empty"
        )));
    }
    Ok(())
}

fn unix_now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CasChunkHash, EnvironmentKind, SnapshotMetadata, SnapshotSidecar, StatusCounts,
        StoredSnapshot, TaskArtifactEntry,
    };

    #[test]
    fn cache_key_includes_snapshot_sidecars_environment_command_and_platform() {
        let execution = execution_snapshot(
            task_definition(Some(TaskCacheMode::ReadWrite)),
            vec![sidecar("large.bin", "sidecar-root-a")],
        );

        let key = task_result_cache_key(&execution, "darwin-arm64", "env-a").unwrap();

        assert!(key.key.starts_with(TASK_RESULT_CACHE_KEY_PREFIX));
        assert_eq!(key.parts.input_head_oid, "head-a");
        assert_eq!(key.parts.input_index_tree_oid, "index-tree-a");
        assert_eq!(key.parts.input_work_tree_oid, "work-tree-a");
        assert_eq!(key.parts.environment_fingerprint, "env-a");
        assert_eq!(key.parts.platform_key, "darwin-arm64");
        assert_eq!(key.parts.command_definition_hash, "c".repeat(64));
        assert_eq!(key.parts.sidecars[0].logical_path, "large.bin");
        assert_eq!(key.parts.sidecars[0].root_hash, "sidecar-root-a");

        let changed_env = task_result_cache_key(&execution, "darwin-arm64", "env-b").unwrap();
        assert_ne!(key.key, changed_env.key);

        let changed_platform = task_result_cache_key(&execution, "linux-x86_64", "env-a").unwrap();
        assert_ne!(key.key, changed_platform.key);

        let mut changed_command = execution.clone();
        changed_command.definition.command_definition_hash = "d".repeat(64);
        assert_ne!(
            key.key,
            task_result_cache_key(&changed_command, "darwin-arm64", "env-a")
                .unwrap()
                .key
        );

        let mut changed_snapshot = execution.clone();
        changed_snapshot.snapshot.metadata.work_tree_oid = "work-tree-b".to_string();
        assert_ne!(
            key.key,
            task_result_cache_key(&changed_snapshot, "darwin-arm64", "env-a")
                .unwrap()
                .key
        );

        let changed_sidecar = execution_snapshot(
            task_definition(Some(TaskCacheMode::ReadWrite)),
            vec![sidecar("large.bin", "sidecar-root-b")],
        );
        assert_ne!(
            key.key,
            task_result_cache_key(&changed_sidecar, "darwin-arm64", "env-a")
                .unwrap()
                .key
        );

        let encoded = serde_json::to_string(&key).unwrap();
        assert!(!encoded.contains("super-secret-value"));
    }

    #[test]
    fn cache_eligibility_respects_modes_and_secret_sensitive_default() {
        let base_manifest = manifest("");
        let read_only = task_result_cache_eligibility(
            &task_definition(Some(TaskCacheMode::Read)),
            &base_manifest,
            TaskResultCachePolicy::default(),
        );
        assert!(read_only.read);
        assert!(!read_only.write);

        let write_only = task_result_cache_eligibility(
            &task_definition(Some(TaskCacheMode::Write)),
            &base_manifest,
            TaskResultCachePolicy::default(),
        );
        assert!(!write_only.read);
        assert!(write_only.write);

        let off = task_result_cache_eligibility(
            &task_definition(None),
            &base_manifest,
            TaskResultCachePolicy::default(),
        );
        assert!(!off.enabled());

        let secret_manifest = manifest(
            r#"
[secrets.api_key]
target = ".secrets/api-key"
"#,
        );
        let disabled = task_result_cache_eligibility(
            &task_definition(Some(TaskCacheMode::ReadWrite)),
            &secret_manifest,
            TaskResultCachePolicy::default(),
        );
        assert!(!disabled.enabled());
        assert!(disabled.secret_sensitive);
        assert!(disabled.explanation.contains("declares secrets"));

        let allowed = task_result_cache_eligibility(
            &task_definition(Some(TaskCacheMode::ReadWrite)),
            &secret_manifest,
            TaskResultCachePolicy {
                allow_secret_sensitive: true,
            },
        );
        assert!(allowed.read);
        assert!(allowed.write);
        assert!(allowed.secret_sensitive);
    }

    #[test]
    fn stores_reads_and_restores_cache_hit_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let platform = crate::current_platform_key();
        let definition = task_definition(Some(TaskCacheMode::ReadWrite));
        let execution = execution_snapshot(definition.clone(), Vec::new());
        let key = task_result_cache_key(&execution, &platform, "env-a").unwrap();
        let eligibility = task_result_cache_eligibility(
            &definition,
            &manifest(""),
            TaskResultCachePolicy::default(),
        );
        let cas = CasStore::open(home.cas_dir(&definition.project_id)).unwrap();
        let artifact_index = artifact_index(&definition, &platform, &cas, b"cached artifact");

        let stored = store_task_result_cache(&home, &key, &artifact_index, &eligibility)
            .unwrap()
            .unwrap();

        assert!(stored.path.exists());
        assert_eq!(
            read_task_result_cache_entry(&stored.path).unwrap(),
            stored.entry
        );

        let hit = lookup_task_result_cache(&home, &key, &eligibility)
            .unwrap()
            .unwrap();
        assert_eq!(hit.entry.key, key.key);

        let restore =
            restore_task_result_cache_hit(&hit, &cas, temp.path().join("restored")).unwrap();
        assert_eq!(restore.restored.len(), 1);
        assert_eq!(restore.total_bytes, 15);
        assert_eq!(
            fs::read_to_string(temp.path().join("restored/dist/app.txt")).unwrap(),
            "cached artifact"
        );

        let write_only = TaskResultCacheEligibility {
            read: false,
            write: true,
            secret_sensitive: false,
            explanation: "write-only".to_string(),
        };
        assert!(
            lookup_task_result_cache(&home, &key, &write_only)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn read_only_and_mismatched_cache_entries_are_not_stored() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let platform = crate::current_platform_key();
        let definition = task_definition(Some(TaskCacheMode::ReadWrite));
        let execution = execution_snapshot(definition.clone(), Vec::new());
        let key = task_result_cache_key(&execution, &platform, "env-a").unwrap();
        let cas = CasStore::open(home.cas_dir(&definition.project_id)).unwrap();
        let mut artifact_index = artifact_index(&definition, &platform, &cas, b"cached artifact");

        let read_only = TaskResultCacheEligibility {
            read: true,
            write: false,
            secret_sensitive: false,
            explanation: "read-only".to_string(),
        };
        assert!(
            store_task_result_cache(&home, &key, &artifact_index, &read_only)
                .unwrap()
                .is_none()
        );

        artifact_index.command_definition_hash = "d".repeat(64);
        let read_write = TaskResultCacheEligibility {
            read: true,
            write: true,
            secret_sensitive: false,
            explanation: "read-write".to_string(),
        };
        let err = store_task_result_cache(&home, &key, &artifact_index, &read_write).unwrap_err();

        assert!(err.to_string().contains("command_definition_hash"));
    }

    fn task_definition(cache: Option<TaskCacheMode>) -> TaskDefinition {
        TaskDefinition {
            project_id: "12345678".to_string(),
            task_name: "build".to_string(),
            profile_name: "dev".to_string(),
            profile_kind: EnvironmentKind::Native,
            command: vec!["cargo".to_string(), "build".to_string()],
            platforms: vec!["darwin-*".to_string()],
            cpu: Some(2),
            memory_mib: Some(1024),
            disk_mib: Some(1024),
            interactive: false,
            cache,
            outputs: vec!["dist/**".to_string()],
            features: vec!["rust".to_string()],
            sandbox: None,
            command_definition_hash: "c".repeat(64),
        }
    }

    fn execution_snapshot(
        definition: TaskDefinition,
        sidecars: Vec<SnapshotSidecar>,
    ) -> TaskExecutionSnapshot {
        TaskExecutionSnapshot {
            definition,
            snapshot: StoredSnapshot {
                snapshot_id: "s1_0123456789abcdef01234567".to_string(),
                project_id: "12345678".to_string(),
                session_id: Some("se_task".to_string()),
                parent_snapshot_id: None,
                sequence_number: 1,
                pinned: true,
                label: Some("task:build:cccccccccccc".to_string()),
                metadata: SnapshotMetadata {
                    schema_version: 1,
                    snapshot_id: "s1_0123456789abcdef01234567".to_string(),
                    project_id: "12345678".to_string(),
                    project_name: "demo".to_string(),
                    session_id: Some("se_task".to_string()),
                    parent_snapshot_id: None,
                    child_snapshots: Vec::new(),
                    source_device_id: Some("dev_a".to_string()),
                    branch: Some("main".to_string()),
                    head_oid: "head-a".to_string(),
                    index_tree_oid: "index-tree-a".to_string(),
                    index_commit_oid: "index-commit-a".to_string(),
                    work_tree_oid: "work-tree-a".to_string(),
                    work_commit_oid: "work-commit-a".to_string(),
                    source_status: StatusCounts::default(),
                    operation_capsule: None,
                    included_untracked: vec!["generated.txt".to_string()],
                    excluded: Vec::new(),
                    sidecars,
                    state_hash: "state-a".to_string(),
                    created_at_unix_seconds: 1,
                },
                created_at_unix_seconds: 1,
            },
            label: "task:build:cccccccccccc".to_string(),
        }
    }

    fn sidecar(logical_path: &str, root_hash: &str) -> SnapshotSidecar {
        SnapshotSidecar {
            logical_path: logical_path.to_string(),
            file_mode: "100644".to_string(),
            classification: "large-file".to_string(),
            size_bytes: 1024,
            chunk_size_bytes: 1024,
            root_hash: root_hash.to_string(),
            cas_manifest_id: root_hash.to_string(),
        }
    }

    fn artifact_index(
        definition: &TaskDefinition,
        platform: &str,
        cas: &CasStore,
        bytes: &[u8],
    ) -> TaskArtifactIndex {
        let chunk = CasChunkHash::from_bytes(bytes);
        cas.upload_chunk(bytes, &chunk).unwrap();
        let manifest = cas.create_manifest(std::slice::from_ref(&chunk)).unwrap();
        cas.add_reachability_root("artifact-tr_cache-0", &manifest.manifest_id)
            .unwrap();
        TaskArtifactIndex {
            task_run_id: "tr_cache".to_string(),
            project_id: definition.project_id.clone(),
            task_name: definition.task_name.clone(),
            platform_key: platform.to_string(),
            command_definition_hash: definition.command_definition_hash.clone(),
            outputs: definition.outputs.clone(),
            entries: vec![TaskArtifactEntry {
                path: "dist/app.txt".to_string(),
                size_bytes: bytes.len() as u64,
                chunk_hash: chunk.as_str().to_string(),
                cas_manifest_id: manifest.manifest_id,
                cas_root_id: "artifact-tr_cache-0".to_string(),
            }],
            missing_outputs: Vec::new(),
            created_at_unix_seconds: 1,
        }
    }

    fn manifest(extra: &str) -> Manifest {
        let raw = format!(
            r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"

{extra}
"#
        );
        Manifest::parse(&raw).unwrap()
    }
}
