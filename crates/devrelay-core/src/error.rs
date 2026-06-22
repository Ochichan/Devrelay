//! Error types shared by the local CLI and core workflows.
//!
//! M0 errors are still intentionally small. Stable, namespaced error codes are
//! tracked separately in the roadmap and should be added before the local CLI
//! MVP depends on machine-readable failures.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum DevRelayError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("glob pattern error: {0}")]
    Glob(#[from] globset::Error),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("manifest validation failed: {0}")]
    Manifest(String),

    #[error("git command failed in {cwd}: git {args}\n{stderr}")]
    GitCommand {
        cwd: PathBuf,
        args: String,
        stderr: String,
    },

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
            Self::Io(_) => "DR-IO",
            Self::Toml(_) | Self::Manifest(_) => "DR-MANIFEST-INVALID",
            Self::Json(_) => "DR-JSON",
            Self::Glob(_) => "DR-GLOB",
            Self::Config(_) => "DR-CONFIG",
            Self::GitCommand { .. } => "DR-GIT-COMMAND",
            Self::TargetDirty(_) => "DR-APPLY-DIRTY-TARGET",
            Self::MissingSourceObject(_) => "DR-APPLY-MISSING-SOURCE-OBJECT",
            Self::SnapshotMetadata(_) => "DR-SNAPSHOT-METADATA",
            Self::UnsupportedRepositoryState(_) => "DR-GIT-UNSUPPORTED-STATE",
            Self::Verification(_) => "DR-APPLY-VERIFICATION-MISMATCH",
        }
    }
}

pub type Result<T> = std::result::Result<T, DevRelayError>;
