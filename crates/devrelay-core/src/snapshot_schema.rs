//! Versioned snapshot metadata schema and state hashing.
//!
//! The schema records semantic Git state separately from transient snapshot
//! storage details. The canonical state hash includes the fields that prove the
//! captured state and intentionally excludes non-semantic fields such as
//! snapshot ID, creation time, project display name, and synthetic commit IDs.

use crate::{ClassifiedPath, DevRelayError, PathDecision, Result, StatusCounts};
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
    pub session_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
    pub source_device_id: Option<String>,
    pub branch: Option<String>,
    pub head_oid: String,
    pub index_tree_oid: String,
    pub index_commit_oid: String,
    pub work_tree_oid: String,
    pub work_commit_oid: String,
    pub source_status: StatusCounts,
    pub included_untracked: Vec<String>,
    pub excluded: Vec<ClassifiedPath>,
    pub state_hash: String,
    pub created_at_unix_seconds: u64,
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

    let mut hasher = blake3::Hasher::new();
    update_hash_field(&mut hasher, "devrelay.state.v1");
    update_hash_field(&mut hasher, &metadata.project_id);
    update_hash_field(&mut hasher, &metadata.head_oid);
    update_hash_field(&mut hasher, &metadata.index_tree_oid);
    update_hash_field(&mut hasher, &metadata.work_tree_oid);
    for path in &included {
        update_hash_field(&mut hasher, "included");
        update_hash_field(&mut hasher, path);
    }
    for item in &excluded {
        update_hash_field(&mut hasher, "excluded");
        update_hash_field(&mut hasher, &item.path);
        update_hash_field(&mut hasher, &item.reason);
    }
    hasher.finalize().to_hex().to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(metadata.source_device_id, None);
        metadata.validate().expect("fixture should validate");
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
            "source_device_id",
            "branch",
            "head_oid",
            "index_tree_oid",
            "index_commit_oid",
            "work_tree_oid",
            "work_commit_oid",
            "source_status",
            "included_untracked",
            "excluded",
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
}
