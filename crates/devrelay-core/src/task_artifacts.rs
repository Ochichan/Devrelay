//! Task output artifact capture and retrieval.
//!
//! Artifacts are declared by task output globs. Captured files are stored in the
//! project CAS and summarized in a per-run artifact index.

use crate::{
    CasStore, DevRelayError, DevRelayHome, Result, TaskDefinition, TaskRunnerWorkspace,
    current_platform_key,
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const TASK_ARTIFACTS_DIR: &str = "artifacts";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskArtifactCaptureSummary {
    pub task_run_id: String,
    pub captured_count: usize,
    pub missing_outputs: Vec<String>,
    pub total_bytes: u64,
    pub index_path: PathBuf,
    pub index: TaskArtifactIndex,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskArtifactIndex {
    pub task_run_id: String,
    pub project_id: String,
    pub task_name: String,
    pub platform_key: String,
    pub command_definition_hash: String,
    pub outputs: Vec<String>,
    pub entries: Vec<TaskArtifactEntry>,
    pub missing_outputs: Vec<String>,
    pub created_at_unix_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskArtifactEntry {
    pub path: String,
    pub size_bytes: u64,
    pub chunk_hash: String,
    pub cas_manifest_id: String,
    pub cas_root_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskArtifactPullResult {
    pub path: String,
    pub destination: PathBuf,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskArtifactRetentionResult {
    pub removed_root_ids: Vec<String>,
}

pub fn capture_task_artifacts(
    home: &DevRelayHome,
    workspace: &TaskRunnerWorkspace,
    definition: &TaskDefinition,
    cas_store: &CasStore,
) -> Result<TaskArtifactCaptureSummary> {
    validate_task_run_id(&workspace.task_run_id)?;
    validate_output_patterns(&definition.outputs)?;
    let output_globs = build_output_globs(&definition.outputs)?;
    let mut matched_patterns = vec![false; definition.outputs.len()];
    let mut entries = Vec::new();
    let mut total_bytes = 0_u64;

    for path in collect_workspace_files(&workspace.path)? {
        let relative = workspace_relative_path(&workspace.path, &path)?;
        let matching_patterns = matching_pattern_indexes(&output_globs, &relative);
        if !matching_patterns.is_empty() {
            for pattern_index in matching_patterns {
                matched_patterns[pattern_index] = true;
            }
            let bytes = fs::read(&path)?;
            let chunk_hash = crate::CasChunkHash::from_bytes(&bytes);
            cas_store.upload_chunk(&bytes, &chunk_hash)?;
            let manifest = cas_store.create_manifest(std::slice::from_ref(&chunk_hash))?;
            let root_id = artifact_root_id(&workspace.task_run_id, entries.len());
            cas_store.add_reachability_root(&root_id, &manifest.manifest_id)?;
            total_bytes = total_bytes.saturating_add(bytes.len() as u64);
            entries.push(TaskArtifactEntry {
                path: relative,
                size_bytes: bytes.len() as u64,
                chunk_hash: chunk_hash.as_str().to_string(),
                cas_manifest_id: manifest.manifest_id,
                cas_root_id: root_id,
            });
        }
    }

    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let missing_outputs = definition
        .outputs
        .iter()
        .enumerate()
        .filter(|(index, _)| !matched_patterns[*index])
        .map(|(_, pattern)| pattern.clone())
        .collect::<Vec<_>>();
    let index = TaskArtifactIndex {
        task_run_id: workspace.task_run_id.clone(),
        project_id: definition.project_id.clone(),
        task_name: definition.task_name.clone(),
        platform_key: current_platform_key(),
        command_definition_hash: definition.command_definition_hash.clone(),
        outputs: definition.outputs.clone(),
        entries,
        missing_outputs: missing_outputs.clone(),
        created_at_unix_seconds: unix_now_seconds(),
    };
    let index_path = task_artifact_index_path(home, &definition.project_id, &workspace.task_run_id);
    write_task_artifact_index(&index_path, &index)?;
    Ok(TaskArtifactCaptureSummary {
        task_run_id: workspace.task_run_id.clone(),
        captured_count: index.entries.len(),
        missing_outputs,
        total_bytes,
        index_path,
        index,
    })
}

pub fn pull_task_artifact(
    index: &TaskArtifactIndex,
    artifact_path: &str,
    cas_store: &CasStore,
    destination_root: impl AsRef<Path>,
) -> Result<TaskArtifactPullResult> {
    let entry = index
        .entries
        .iter()
        .find(|entry| entry.path == artifact_path)
        .ok_or_else(|| DevRelayError::Config(format!("unknown artifact path {artifact_path:?}")))?;
    let relative = safe_relative_path(&entry.path)?;
    let destination = destination_root.as_ref().join(relative);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let manifest = cas_store.fetch_manifest(&entry.cas_manifest_id)?;
    let mut bytes = Vec::new();
    for chunk in manifest.chunks {
        bytes.extend(cas_store.download_chunk(&chunk.hash)?);
    }
    fs::write(&destination, &bytes)?;
    Ok(TaskArtifactPullResult {
        path: entry.path.clone(),
        destination,
        size_bytes: bytes.len() as u64,
    })
}

pub fn apply_task_artifact_retention(
    index: &TaskArtifactIndex,
    cas_store: &CasStore,
) -> Result<TaskArtifactRetentionResult> {
    let mut removed_root_ids = Vec::new();
    for entry in &index.entries {
        if cas_store.remove_reachability_root(&entry.cas_root_id)? {
            removed_root_ids.push(entry.cas_root_id.clone());
        }
    }
    Ok(TaskArtifactRetentionResult { removed_root_ids })
}

pub fn read_task_artifact_index(path: impl AsRef<Path>) -> Result<TaskArtifactIndex> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

pub fn task_artifact_index_path(
    home: &DevRelayHome,
    project_id: &str,
    task_run_id: &str,
) -> PathBuf {
    home.project_data_dir(project_id)
        .join(TASK_ARTIFACTS_DIR)
        .join(format!("{task_run_id}.json"))
}

fn write_task_artifact_index(path: &Path, index: &TaskArtifactIndex) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(index)?)?;
    Ok(())
}

fn validate_output_patterns(outputs: &[String]) -> Result<()> {
    for output in outputs {
        safe_relative_path(output)?;
    }
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

fn safe_relative_path(path: &str) -> Result<&Path> {
    let relative = Path::new(path);
    if path.trim().is_empty()
        || relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(DevRelayError::Config(format!(
            "artifact path must stay inside the runner workspace: {path:?}"
        )));
    }
    Ok(relative)
}

fn build_output_globs(outputs: &[String]) -> Result<Vec<GlobSet>> {
    outputs
        .iter()
        .map(|output| {
            let mut builder = GlobSetBuilder::new();
            builder.add(Glob::new(output)?);
            Ok(builder.build()?)
        })
        .collect()
}

fn matching_pattern_indexes(globs: &[GlobSet], relative: &str) -> Vec<usize> {
    globs
        .iter()
        .enumerate()
        .filter_map(|(index, glob)| glob.is_match(relative).then_some(index))
        .collect()
}

fn collect_workspace_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_workspace_files_inner(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_workspace_files_inner(path: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_workspace_files_inner(&entry.path(), files)?;
        } else if file_type.is_file() {
            files.push(entry.path());
        }
    }
    Ok(())
}

fn workspace_relative_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|err| DevRelayError::Config(format!("artifact path escaped workspace: {err}")))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn artifact_root_id(task_run_id: &str, index: usize) -> String {
    format!("artifact-{task_run_id}-{index}")
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
        EnvironmentKind, TaskCacheMode, TaskRunnerEnvironmentState, TaskRunnerSecretState,
        TaskRunnerSidecarState, TaskRunnerWorkspace, TaskRunnerWorkspaceRetentionPolicy,
        VerificationDetails,
    };

    #[test]
    fn captures_declared_outputs_into_cas_and_index() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let workspace_path = temp.path().join("workspace");
        fs::create_dir_all(workspace_path.join("dist/nested")).unwrap();
        fs::write(workspace_path.join("dist/app.txt"), "artifact").unwrap();
        fs::write(workspace_path.join("dist/nested/extra.log"), "extra").unwrap();
        fs::write(workspace_path.join("ignored.txt"), "ignored").unwrap();
        let cas = CasStore::open(home.cas_dir("12345678")).unwrap();
        let workspace = workspace(&workspace_path);
        let definition = task_definition(vec!["dist/**".to_string(), "missing/**".to_string()]);

        let summary = capture_task_artifacts(&home, &workspace, &definition, &cas).unwrap();

        assert_eq!(summary.captured_count, 2);
        assert_eq!(summary.missing_outputs, vec!["missing/**"]);
        assert_eq!(summary.total_bytes, 13);
        assert!(summary.index_path.exists());
        assert_eq!(
            summary
                .index
                .entries
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["dist/app.txt", "dist/nested/extra.log"]
        );
        assert_eq!(
            read_task_artifact_index(&summary.index_path).unwrap(),
            summary.index
        );
        let first = &summary.index.entries[0];
        assert!(cas.fetch_manifest(&first.cas_manifest_id).is_ok());
        assert!(cas.fetch_reachability_root(&first.cas_root_id).is_ok());
    }

    #[test]
    fn pulls_artifact_on_demand_and_applies_retention_roots() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let workspace_path = temp.path().join("workspace");
        fs::create_dir_all(workspace_path.join("dist")).unwrap();
        fs::write(workspace_path.join("dist/app.txt"), "artifact").unwrap();
        let cas = CasStore::open(home.cas_dir("12345678")).unwrap();
        let workspace = workspace(&workspace_path);
        let definition = task_definition(vec!["dist/**".to_string()]);
        let summary = capture_task_artifacts(&home, &workspace, &definition, &cas).unwrap();

        let pull = pull_task_artifact(
            &summary.index,
            "dist/app.txt",
            &cas,
            temp.path().join("pulled"),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&pull.destination).unwrap(), "artifact");

        let retention = apply_task_artifact_retention(&summary.index, &cas).unwrap();
        assert_eq!(retention.removed_root_ids.len(), 1);
        assert!(
            cas.fetch_reachability_root(&summary.index.entries[0].cas_root_id)
                .is_err()
        );
    }

    #[test]
    fn treats_overlapping_output_patterns_as_present() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let workspace_path = temp.path().join("workspace");
        fs::create_dir_all(workspace_path.join("dist")).unwrap();
        fs::write(workspace_path.join("dist/app.txt"), "artifact").unwrap();
        let cas = CasStore::open(home.cas_dir("12345678")).unwrap();
        let workspace = workspace(&workspace_path);
        let definition = task_definition(vec!["dist/**".to_string(), "dist/app.txt".to_string()]);

        let summary = capture_task_artifacts(&home, &workspace, &definition, &cas).unwrap();

        assert_eq!(summary.captured_count, 1);
        assert!(summary.missing_outputs.is_empty());
    }

    #[test]
    fn rejects_output_path_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let workspace_path = temp.path().join("workspace");
        fs::create_dir_all(&workspace_path).unwrap();
        let cas = CasStore::open(home.cas_dir("12345678")).unwrap();
        let workspace = workspace(&workspace_path);
        let definition = task_definition(vec!["../secret".to_string()]);

        let err = capture_task_artifacts(&home, &workspace, &definition, &cas).unwrap_err();

        assert!(err.to_string().contains("artifact path"));
    }

    #[test]
    fn rejects_unsafe_task_run_ids_for_artifact_index_paths() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let workspace_path = temp.path().join("workspace");
        fs::create_dir_all(&workspace_path).unwrap();
        let cas = CasStore::open(home.cas_dir("12345678")).unwrap();
        let mut workspace = workspace(&workspace_path);
        workspace.task_run_id = "../escape".to_string();
        let definition = task_definition(vec!["dist/**".to_string()]);

        let err = capture_task_artifacts(&home, &workspace, &definition, &cas).unwrap_err();

        assert!(err.to_string().contains("invalid task run id"));
    }

    fn task_definition(outputs: Vec<String>) -> TaskDefinition {
        TaskDefinition {
            project_id: "12345678".to_string(),
            task_name: "build".to_string(),
            profile_name: "dev".to_string(),
            profile_kind: EnvironmentKind::Native,
            command: vec!["fake".to_string()],
            platforms: vec!["darwin-*".to_string()],
            cpu: Some(1),
            memory_mib: Some(64),
            disk_mib: Some(64),
            interactive: false,
            cache: Some(TaskCacheMode::ReadWrite),
            outputs,
            features: Vec::new(),
            sandbox: None,
            command_definition_hash: "c".repeat(64),
        }
    }

    fn workspace(path: &Path) -> TaskRunnerWorkspace {
        TaskRunnerWorkspace {
            task_run_id: "tr_artifacts".to_string(),
            project_id: "12345678".to_string(),
            task_name: "build".to_string(),
            path: path.to_path_buf(),
            snapshot_id: "snap_123".to_string(),
            canonical_session: false,
            environment: TaskRunnerEnvironmentState {
                profile_name: "dev".to_string(),
                kind: EnvironmentKind::Native,
                command_scope: "environment.profile.dev".to_string(),
                hydrated: true,
                explanation: Vec::new(),
            },
            sidecars: TaskRunnerSidecarState::NotRequired,
            secrets: TaskRunnerSecretState::SkippedNotPermitted {
                required: Vec::new(),
            },
            verification: VerificationDetails {
                head_oid: "h".to_string(),
                index_tree_oid: "i".to_string(),
                work_tree_oid: "w".to_string(),
                state_hash: "s".to_string(),
                included_untracked: Vec::new(),
                excluded_paths: Vec::new(),
            },
            retention_policy: TaskRunnerWorkspaceRetentionPolicy::delete_on_cleanup(),
        }
    }
}
