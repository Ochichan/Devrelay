//! Error types shared by the local CLI and core workflows.
//!
//! DevRelay errors carry stable machine-readable codes plus short human titles,
//! details, and safe actions. The CLI adds per-render diagnostic IDs.

use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ErrorInfo {
    pub code: &'static str,
    pub title: &'static str,
    pub detail: String,
    pub safe_actions: Vec<&'static str>,
}

#[derive(Debug, thiserror::Error)]
pub enum DevRelayError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("TOML serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("glob pattern error: {0}")]
    Glob(#[from] globset::Error),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("recovery error: {0}")]
    Recover(String),

    #[error("manifest validation failed: {0}")]
    Manifest(String),

    #[error("git command failed in {cwd}: git {args}\n{stderr}")]
    GitCommand {
        cwd: PathBuf,
        args: String,
        stderr: String,
    },

    #[error("path is not a Git repository: {0}")]
    NotGitRepository(String),

    #[error("target workspace is dirty: {0}")]
    TargetDirty(String),

    #[error("missing source snapshot object: {0}")]
    MissingSourceObject(String),

    #[error("snapshot metadata validation failed: {0}")]
    SnapshotMetadata(String),

    #[error("unsupported repository state: {0}")]
    UnsupportedRepositoryState(String),

    #[error("snapshot verification failed: {0}")]
    Verification(String),
}

impl DevRelayError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "DR-STORAGE-IO",
            Self::Toml(_) | Self::TomlSerialize(_) | Self::Manifest(_) => "DR-MANIFEST-INVALID",
            Self::Json(_) => "DR-SNAPSHOT-JSON",
            Self::Sqlite(_) => "DR-STORAGE-SQLITE",
            Self::Glob(_) => "DR-MANIFEST-GLOB",
            Self::Config(_) => "DR-CONFIG",
            Self::Recover(_) => "DR-RECOVER-SNAPSHOT-NOT-FOUND",
            Self::GitCommand { .. } => "DR-GIT-COMMAND",
            Self::NotGitRepository(_) => "DR-GIT-NOT-REPOSITORY",
            Self::TargetDirty(_) => "DR-APPLY-DIRTY-TARGET",
            Self::MissingSourceObject(_) => "DR-APPLY-MISSING-SOURCE-OBJECT",
            Self::SnapshotMetadata(_) => "DR-SNAPSHOT-METADATA",
            Self::UnsupportedRepositoryState(_) => "DR-GIT-UNSUPPORTED-STATE",
            Self::Verification(_) => "DR-APPLY-VERIFICATION-MISMATCH",
        }
    }

    pub fn info(&self) -> ErrorInfo {
        let (title, safe_actions) = match self {
            Self::Io(_) => (
                "Storage I/O error",
                vec!["Check that the path exists and that DevRelay can read or write it."],
            ),
            Self::Toml(_) | Self::TomlSerialize(_) | Self::Manifest(_) => (
                "Invalid manifest",
                vec!["Check devrelay.toml syntax and required manifest fields."],
            ),
            Self::Json(_) | Self::SnapshotMetadata(_) => (
                "Invalid snapshot metadata",
                vec!["Export the snapshot again or choose a different snapshot ID."],
            ),
            Self::Sqlite(_) => (
                "Metadata store error",
                vec!["Check DEVRELAY_HOME permissions and retry the command."],
            ),
            Self::Glob(_) => (
                "Invalid manifest glob",
                vec!["Fix the manifest include or exclude pattern and retry."],
            ),
            Self::Config(_) => (
                "Invalid local configuration",
                vec![
                    "Run the command with the intended --config path and inspect the config file.",
                ],
            ),
            Self::Recover(_) => (
                "Recovery snapshot not found",
                vec!["Run devrelay recover list and verify the project or snapshot ID."],
            ),
            Self::GitCommand { .. } => (
                "Git command failed",
                vec!["Check the repository path and Git output, then retry."],
            ),
            Self::NotGitRepository(_) => (
                "Not a Git repository",
                vec!["Choose a path inside an initialized Git repository."],
            ),
            Self::TargetDirty(_) => (
                "Target workspace is dirty",
                vec![
                    "Commit, stash, or clean the target, or use --dirty-policy snapshot-and-fork.",
                ],
            ),
            Self::MissingSourceObject(_) => (
                "Missing source snapshot object",
                vec!["Use the snapshot store path or export the snapshot refs before applying."],
            ),
            Self::UnsupportedRepositoryState(_) => (
                "Unsupported Git state",
                vec!["Finish the in-progress Git operation and retry."],
            ),
            Self::Verification(_) => (
                "Snapshot verification failed",
                vec![
                    "Do not continue with the recovered workspace until the mismatch is resolved.",
                ],
            ),
        };
        ErrorInfo {
            code: self.code(),
            title,
            detail: self.to_string(),
            safe_actions,
        }
    }
}

pub type Result<T> = std::result::Result<T, DevRelayError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn representative_errors_have_namespaced_codes_and_actions() {
        let cases = [
            (
                DevRelayError::Manifest("bad schema".to_string()),
                "DR-MANIFEST-INVALID",
            ),
            (
                DevRelayError::GitCommand {
                    cwd: "/tmp/repo".into(),
                    args: "status".to_string(),
                    stderr: "fatal".to_string(),
                },
                "DR-GIT-COMMAND",
            ),
            (
                DevRelayError::SnapshotMetadata("missing field".to_string()),
                "DR-SNAPSHOT-METADATA",
            ),
            (
                DevRelayError::TargetDirty("1 unstaged".to_string()),
                "DR-APPLY-DIRTY-TARGET",
            ),
            (
                DevRelayError::Recover("unknown snapshot".to_string()),
                "DR-RECOVER-SNAPSHOT-NOT-FOUND",
            ),
            (
                DevRelayError::Sqlite(rusqlite::Error::InvalidQuery),
                "DR-STORAGE-SQLITE",
            ),
        ];

        for (err, expected_code) in cases {
            let info = err.info();
            assert_eq!(info.code, expected_code);
            assert!(!info.title.is_empty());
            assert!(!info.detail.is_empty());
            assert!(!info.safe_actions.is_empty());
        }
    }

    #[test]
    fn recovery_errors_use_recover_namespace() {
        let info = DevRelayError::Recover("unknown snapshot s1_x".to_string()).info();

        assert_eq!(info.code, "DR-RECOVER-SNAPSHOT-NOT-FOUND");
        assert_eq!(info.title, "Recovery snapshot not found");
    }
}
