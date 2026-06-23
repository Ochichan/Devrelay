//! Versioned snapshot metadata schema and state hashing.
//!
//! The schema records semantic Git state separately from transient snapshot
//! storage details. The canonical state hash includes the fields that prove the
//! captured state and intentionally excludes non-semantic fields such as
//! snapshot ID, creation time, project display name, and synthetic commit IDs.

use crate::{
    ClassifiedPath, DevRelayError, GitOperationKind, OperationCapsule, PathDecision, Result,
    StatusCounts,
};
use serde::{Deserialize, Serialize};

pub(crate) const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub(crate) const SNAPSHOT_ID_PREFIX: &str = "s1_";
const SNAPSHOT_ID_HEX_LEN: usize = 24;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotMetadata {
    pub schema_version: u32,
    pub snapshot_id: String,
    pub project_id: String,
    pub project_name: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub parent_snapshot_id: Option<String>,
    #[serde(default)]
    pub child_snapshots: Vec<SnapshotChildSnapshot>,
    #[serde(default)]
    pub source_device_id: Option<String>,
    pub branch: Option<String>,
    pub head_oid: String,
    pub index_tree_oid: String,
    pub index_commit_oid: String,
    pub work_tree_oid: String,
    pub work_commit_oid: String,
    pub source_status: StatusCounts,
    #[serde(default)]
    pub operation_capsule: Option<OperationCapsule>,
    pub included_untracked: Vec<String>,
    pub excluded: Vec<ClassifiedPath>,
    #[serde(default)]
    pub sidecars: Vec<SnapshotSidecar>,
    pub state_hash: String,
    pub created_at_unix_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotSidecar {
    pub logical_path: String,
    pub file_mode: String,
    pub classification: String,
    pub size_bytes: u64,
    pub chunk_size_bytes: u64,
    pub root_hash: String,
    pub cas_manifest_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotChildSnapshot {
    pub relationship: String,
    pub path: String,
    pub project_id: String,
    pub session_id: String,
    pub snapshot_id: String,
    pub state_hash: String,
}

impl SnapshotMetadata {
    pub fn index_ref(&self) -> String {
        format!("refs/devrelay/snapshots/{}/index", self.snapshot_id)
    }

    pub fn work_ref(&self) -> String {
        format!("refs/devrelay/snapshots/{}/work", self.snapshot_id)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SNAPSHOT_SCHEMA_VERSION {
            return Err(DevRelayError::SnapshotMetadata(format!(
                "unsupported snapshot schema {}, expected {}",
                self.schema_version, SNAPSHOT_SCHEMA_VERSION
            )));
        }
        if !is_valid_snapshot_id(&self.snapshot_id) {
            return Err(DevRelayError::SnapshotMetadata(format!(
                "malformed snapshot_id {}",
                self.snapshot_id
            )));
        }
        for (field, value) in [
            ("project_id", self.project_id.as_str()),
            ("project_name", self.project_name.as_str()),
            ("head_oid", self.head_oid.as_str()),
            ("index_tree_oid", self.index_tree_oid.as_str()),
            ("index_commit_oid", self.index_commit_oid.as_str()),
            ("work_tree_oid", self.work_tree_oid.as_str()),
            ("work_commit_oid", self.work_commit_oid.as_str()),
            ("state_hash", self.state_hash.as_str()),
        ] {
            if value.is_empty() {
                return Err(DevRelayError::SnapshotMetadata(format!(
                    "{field} must not be empty"
                )));
            }
        }
        for sidecar in &self.sidecars {
            sidecar.validate()?;
        }
        for child in &self.child_snapshots {
            child.validate()?;
        }
        Ok(())
    }
}

impl SnapshotChildSnapshot {
    pub fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("child_snapshots.relationship", self.relationship.as_str()),
            ("child_snapshots.path", self.path.as_str()),
            ("child_snapshots.project_id", self.project_id.as_str()),
            ("child_snapshots.session_id", self.session_id.as_str()),
            ("child_snapshots.snapshot_id", self.snapshot_id.as_str()),
            ("child_snapshots.state_hash", self.state_hash.as_str()),
        ] {
            if value.is_empty() {
                return Err(DevRelayError::SnapshotMetadata(format!(
                    "{field} must not be empty"
                )));
            }
        }
        if self.path.starts_with('/')
            || self.path.contains('\\')
            || self.path.split('/').any(|part| part == "..")
        {
            return Err(DevRelayError::SnapshotMetadata(format!(
                "child snapshot path {} must be repository-relative",
                self.path
            )));
        }
        if !is_valid_snapshot_id(&self.snapshot_id) {
            return Err(DevRelayError::SnapshotMetadata(format!(
                "child snapshot {} has malformed snapshot_id {}",
                self.path, self.snapshot_id
            )));
        }
        Ok(())
    }
}

impl SnapshotSidecar {
    pub fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("sidecar.logical_path", self.logical_path.as_str()),
            ("sidecar.file_mode", self.file_mode.as_str()),
            ("sidecar.classification", self.classification.as_str()),
            ("sidecar.root_hash", self.root_hash.as_str()),
            ("sidecar.cas_manifest_id", self.cas_manifest_id.as_str()),
        ] {
            if value.is_empty() {
                return Err(DevRelayError::SnapshotMetadata(format!(
                    "{field} must not be empty"
                )));
            }
        }
        if self.logical_path.starts_with('/')
            || self.logical_path.contains('\\')
            || self.logical_path.split('/').any(|part| part == "..")
        {
            return Err(DevRelayError::SnapshotMetadata(format!(
                "sidecar logical_path {} must be repository-relative",
                self.logical_path
            )));
        }
        if self.size_bytes == 0 {
            return Err(DevRelayError::SnapshotMetadata(format!(
                "sidecar {} size_bytes must be positive",
                self.logical_path
            )));
        }
        if self.chunk_size_bytes == 0 {
            return Err(DevRelayError::SnapshotMetadata(format!(
                "sidecar {} chunk_size_bytes must be positive",
                self.logical_path
            )));
        }
        if self.root_hash != self.cas_manifest_id {
            return Err(DevRelayError::SnapshotMetadata(format!(
                "sidecar {} root_hash must match cas_manifest_id",
                self.logical_path
            )));
        }
        Ok(())
    }
}

pub(crate) fn is_valid_snapshot_id(snapshot_id: &str) -> bool {
    let Some(hex) = snapshot_id.strip_prefix(SNAPSHOT_ID_PREFIX) else {
        return false;
    };
    hex.len() == SNAPSHOT_ID_HEX_LEN && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(crate) fn calculate_state_hash(metadata: &SnapshotMetadata) -> String {
    let mut included = metadata.included_untracked.clone();
    included.sort();

    let mut excluded = metadata.excluded.clone();
    excluded.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.reason.cmp(&right.reason))
            .then(decision_order(left.decision).cmp(&decision_order(right.decision)))
    });

    let mut sidecars = metadata.sidecars.clone();
    sidecars.sort_by(|left, right| {
        left.logical_path
            .cmp(&right.logical_path)
            .then(left.cas_manifest_id.cmp(&right.cas_manifest_id))
    });

    let mut child_snapshots = metadata.child_snapshots.clone();
    child_snapshots.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.relationship.cmp(&right.relationship))
            .then(left.project_id.cmp(&right.project_id))
            .then(left.session_id.cmp(&right.session_id))
            .then(left.snapshot_id.cmp(&right.snapshot_id))
    });

    let mut hasher = blake3::Hasher::new();
    update_hash_field(&mut hasher, "devrelay.state.v1");
    update_hash_field(&mut hasher, &metadata.project_id);
    update_hash_field(&mut hasher, &metadata.head_oid);
    update_hash_field(&mut hasher, &metadata.index_tree_oid);
    update_hash_field(&mut hasher, &metadata.work_tree_oid);
    if let Some(capsule) = &metadata.operation_capsule {
        update_operation_capsule_hash(&mut hasher, capsule);
    }
    for path in &included {
        update_hash_field(&mut hasher, "included");
        update_hash_field(&mut hasher, path);
    }
    for item in &excluded {
        update_hash_field(&mut hasher, "excluded");
        update_hash_field(&mut hasher, &item.path);
        update_hash_field(&mut hasher, &item.reason);
    }
    for sidecar in &sidecars {
        update_hash_field(&mut hasher, "sidecar");
        update_hash_field(&mut hasher, &sidecar.logical_path);
        update_hash_field(&mut hasher, &sidecar.file_mode);
        update_hash_field(&mut hasher, &sidecar.classification);
        update_hash_field(&mut hasher, &sidecar.size_bytes.to_string());
        update_hash_field(&mut hasher, &sidecar.chunk_size_bytes.to_string());
        update_hash_field(&mut hasher, &sidecar.root_hash);
        update_hash_field(&mut hasher, &sidecar.cas_manifest_id);
    }
    for child in &child_snapshots {
        update_hash_field(&mut hasher, "child-snapshot");
        update_hash_field(&mut hasher, &child.relationship);
        update_hash_field(&mut hasher, &child.path);
        update_hash_field(&mut hasher, &child.project_id);
        update_hash_field(&mut hasher, &child.session_id);
        update_hash_field(&mut hasher, &child.snapshot_id);
        update_hash_field(&mut hasher, &child.state_hash);
    }
    hasher.finalize().to_hex().to_string()
}

fn update_operation_capsule_hash(hasher: &mut blake3::Hasher, capsule: &OperationCapsule) {
    update_hash_field(hasher, "operation-capsule");
    update_hash_field(hasher, operation_kind_label(capsule.operation.kind));
    update_hash_field(hasher, &capsule.operation.current_head_oid);
    for oid in &capsule.operation.operation_oids {
        update_hash_field(hasher, "operation-oid");
        update_hash_field(hasher, oid);
    }
    if let Some(oid) = &capsule.operation.original_head_oid {
        update_hash_field(hasher, "original-head");
        update_hash_field(hasher, oid);
    }
    if let Some(progress) = &capsule.operation.progress {
        update_hash_field(hasher, "operation-progress");
        update_hash_field(
            hasher,
            if progress.interactive {
                "true"
            } else {
                "false"
            },
        );
        if let Some(oid) = &progress.original_head_oid {
            update_hash_field(hasher, "progress-original-head");
            update_hash_field(hasher, oid);
        }
        if let Some(oid) = &progress.onto_oid {
            update_hash_field(hasher, "progress-onto");
            update_hash_field(hasher, oid);
        }
        if let Some(head_name) = &progress.head_name {
            update_hash_field(hasher, "progress-head-name");
            update_hash_field(hasher, head_name);
        }
        for todo in &progress.todo {
            update_hash_field(hasher, "progress-todo");
            update_hash_field(hasher, todo);
        }
        for done in &progress.done {
            update_hash_field(hasher, "progress-done");
            update_hash_field(hasher, done);
        }
        if let Some(step) = &progress.current_step {
            update_hash_field(hasher, "progress-current-step");
            if let Some(current) = step.current {
                update_hash_field(hasher, "current");
                update_hash_field(hasher, &current.to_string());
            }
            if let Some(total) = step.total {
                update_hash_field(hasher, "total");
                update_hash_field(hasher, &total.to_string());
            }
        }
    }
    for entry in &capsule.unmerged_entries {
        update_hash_field(hasher, "unmerged-entry");
        update_hash_field(hasher, &entry.path);
        for stage in &entry.stages {
            update_hash_field(hasher, "stage");
            update_hash_field(hasher, &stage.stage.to_string());
            update_hash_field(hasher, &stage.mode);
            update_hash_field(hasher, &stage.oid);
        }
    }
    for file in &capsule.worktree_files {
        update_hash_field(hasher, "conflict-worktree-file");
        update_hash_field(hasher, &file.path);
        update_hash_bytes(hasher, &file.contents);
    }
}

fn operation_kind_label(kind: GitOperationKind) -> &'static str {
    match kind {
        GitOperationKind::Merge => "merge",
        GitOperationKind::CherryPick => "cherry-pick",
        GitOperationKind::Revert => "revert",
        GitOperationKind::RebaseMerge => "rebase-merge",
        GitOperationKind::RebaseApply => "rebase-apply",
        GitOperationKind::Sequencer => "sequencer",
    }
}

fn decision_order(decision: PathDecision) -> u8 {
    match decision {
        PathDecision::Include => 0,
        PathDecision::Exclude => 1,
    }
}

fn update_hash_field(hasher: &mut blake3::Hasher, value: &str) {
    hasher.update(value.as_bytes());
    hasher.update(&[0]);
}

fn update_hash_bytes(hasher: &mut blake3::Hasher, value: &[u8]) {
    hasher.update(&value.len().to_le_bytes());
    hasher.update(value);
    hasher.update(&[0]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GitOperationMetadata, IndexStageEntry, UnmergedIndexEntry};

    fn metadata() -> SnapshotMetadata {
        let mut metadata: SnapshotMetadata =
            serde_json::from_str(include_str!("../tests/fixtures/snapshot_metadata_v1.json"))
                .expect("fixture should deserialize");
        metadata.state_hash = calculate_state_hash(&metadata);
        metadata
    }

    #[test]
    fn deserializes_current_schema_fixture() {
        let metadata: SnapshotMetadata =
            serde_json::from_str(include_str!("../tests/fixtures/snapshot_metadata_v1.json"))
                .expect("fixture should deserialize");
        assert_eq!(metadata.schema_version, SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(metadata.session_id, None);
        assert_eq!(metadata.parent_snapshot_id, None);
        assert!(metadata.child_snapshots.is_empty());
        assert_eq!(metadata.source_device_id, None);
        assert_eq!(metadata.operation_capsule, None);
        metadata.validate().expect("fixture should validate");
    }

    #[test]
    fn migrates_legacy_v1_fixture_without_optional_context_fields() {
        let metadata: SnapshotMetadata = serde_json::from_str(include_str!(
            "../tests/fixtures/snapshot_metadata_v1_legacy_minimal.json"
        ))
        .expect("legacy fixture should deserialize");

        assert_eq!(metadata.schema_version, SNAPSHOT_SCHEMA_VERSION);
        assert_eq!(metadata.session_id, None);
        assert_eq!(metadata.parent_snapshot_id, None);
        assert!(metadata.child_snapshots.is_empty());
        assert_eq!(metadata.source_device_id, None);
        assert_eq!(metadata.operation_capsule, None);
        metadata.validate().expect("legacy fixture should validate");
    }

    #[test]
    fn serializes_with_stable_field_names() {
        let metadata = metadata();
        let json = serde_json::to_string(&metadata).expect("metadata should serialize");
        let fields = [
            "schema_version",
            "snapshot_id",
            "project_id",
            "project_name",
            "session_id",
            "parent_snapshot_id",
            "child_snapshots",
            "source_device_id",
            "branch",
            "head_oid",
            "index_tree_oid",
            "index_commit_oid",
            "work_tree_oid",
            "work_commit_oid",
            "source_status",
            "operation_capsule",
            "included_untracked",
            "excluded",
            "sidecars",
            "state_hash",
            "created_at_unix_seconds",
        ];

        let mut previous = 0;
        for field in fields {
            let needle = format!("\"{field}\"");
            let index = json.find(&needle).expect("field should be serialized");
            assert!(index >= previous, "field order regressed at {field}");
            previous = index;
        }
    }

    #[test]
    fn rejects_empty_required_oids() {
        let mut metadata = metadata();
        metadata.head_oid.clear();
        let err = metadata.validate().expect_err("empty head oid should fail");
        assert!(err.to_string().contains("head_oid"));
    }

    #[test]
    fn rejects_malformed_snapshot_ids() {
        let mut metadata = metadata();
        metadata.snapshot_id = "bad".to_string();
        let err = metadata
            .validate()
            .expect_err("bad snapshot id should fail");
        assert!(err.to_string().contains("malformed snapshot_id"));
    }

    #[test]
    fn validates_snapshot_id_format() {
        assert!(is_valid_snapshot_id("s1_0123456789abcdef01234567"));
        assert!(!is_valid_snapshot_id("s_0123456789abcdef01234567"));
        assert!(!is_valid_snapshot_id("s1_0123456789abcdef0123456"));
        assert!(!is_valid_snapshot_id("s1_0123456789abcdef0123456x"));
    }

    #[test]
    fn state_hash_is_independent_of_path_order() {
        let mut first = metadata();
        first.included_untracked = vec!["b.txt".to_string(), "a.txt".to_string()];
        first.excluded = vec![
            ClassifiedPath {
                path: ".env".to_string(),
                decision: PathDecision::Exclude,
                reason: "secret-filename".to_string(),
            },
            ClassifiedPath {
                path: "target/app".to_string(),
                decision: PathDecision::Exclude,
                reason: "manifest-or-generated-exclude".to_string(),
            },
        ];

        let mut second = first.clone();
        second.included_untracked.reverse();
        second.excluded.reverse();

        assert_eq!(calculate_state_hash(&first), calculate_state_hash(&second));
    }

    #[test]
    fn state_hash_changes_when_tree_content_changes() {
        let first = metadata();
        let mut second = first.clone();
        second.work_tree_oid = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_string();

        assert_ne!(calculate_state_hash(&first), calculate_state_hash(&second));
    }

    #[test]
    fn state_hash_changes_when_sidecar_manifest_changes() {
        let first = metadata();
        let mut second = first.clone();
        second.sidecars.push(SnapshotSidecar {
            logical_path: "large.bin".to_string(),
            file_mode: "100644".to_string(),
            classification: "large-file-threshold".to_string(),
            size_bytes: 1024,
            chunk_size_bytes: 512,
            root_hash: "b3_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
            cas_manifest_id: "b3_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                .to_string(),
        });

        assert_ne!(calculate_state_hash(&first), calculate_state_hash(&second));
        second.state_hash = calculate_state_hash(&second);
        second.validate().unwrap();
    }

    #[test]
    fn state_hash_changes_when_child_snapshot_topology_changes() {
        let first = metadata();
        let mut second = first.clone();
        second
            .child_snapshots
            .push(sample_child_snapshot("deps/lib"));

        assert_ne!(calculate_state_hash(&first), calculate_state_hash(&second));
        second.state_hash = calculate_state_hash(&second);
        second.validate().unwrap();
    }

    #[test]
    fn rejects_child_snapshot_path_traversal() {
        let mut metadata = metadata();
        metadata.child_snapshots.push(SnapshotChildSnapshot {
            path: "../escape".to_string(),
            ..sample_child_snapshot("deps/lib")
        });

        let err = metadata
            .validate()
            .expect_err("child snapshot traversal should fail");

        assert!(err.to_string().contains("repository-relative"));
    }

    #[test]
    fn operation_capsule_round_trips_and_contributes_to_state_hash() {
        let mut first = metadata();
        first.operation_capsule = Some(sample_operation_capsule("stage-one"));
        let mut second = first.clone();
        second.operation_capsule.as_mut().unwrap().worktree_files[0].contents =
            b"changed conflict markers\n".to_vec();

        assert_ne!(calculate_state_hash(&first), calculate_state_hash(&second));

        first.state_hash = calculate_state_hash(&first);
        let encoded = serde_json::to_string(&first).expect("metadata should serialize");
        let decoded: SnapshotMetadata =
            serde_json::from_str(&encoded).expect("metadata should deserialize");

        assert_eq!(decoded.operation_capsule, first.operation_capsule);
        assert!(encoded.contains("\"operation_capsule\""));
    }

    fn sample_operation_capsule(stage_oid: &str) -> OperationCapsule {
        OperationCapsule {
            operation: GitOperationMetadata {
                kind: GitOperationKind::Merge,
                current_head_oid: "1111111111111111111111111111111111111111".to_string(),
                operation_oids: vec!["2222222222222222222222222222222222222222".to_string()],
                original_head_oid: Some("3333333333333333333333333333333333333333".to_string()),
                progress: None,
            },
            unmerged_entries: vec![UnmergedIndexEntry {
                path: "conflict.txt".to_string(),
                stages: vec![
                    IndexStageEntry {
                        stage: 1,
                        mode: "100644".to_string(),
                        oid: stage_oid.to_string(),
                    },
                    IndexStageEntry {
                        stage: 2,
                        mode: "100644".to_string(),
                        oid: "ours".to_string(),
                    },
                    IndexStageEntry {
                        stage: 3,
                        mode: "100644".to_string(),
                        oid: "theirs".to_string(),
                    },
                ],
            }],
            worktree_files: vec![crate::ConflictWorktreeFile {
                path: "conflict.txt".to_string(),
                contents: format!("<<<<<<< HEAD\n{stage_oid}\n=======\ntheirs\n>>>>>>> branch\n")
                    .into_bytes(),
            }],
        }
    }

    fn sample_child_snapshot(path: &str) -> SnapshotChildSnapshot {
        SnapshotChildSnapshot {
            relationship: "git-submodule".to_string(),
            path: path.to_string(),
            project_id: "child-project".to_string(),
            session_id: "se_0123456789abcdef01234567".to_string(),
            snapshot_id: "s1_0123456789abcdef01234567".to_string(),
            state_hash: "child-state-hash".to_string(),
        }
    }
}
