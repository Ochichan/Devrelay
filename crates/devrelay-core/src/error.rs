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

    #[error("snapshot metadata validation failed: {0}")]
    SnapshotMetadata(String),

    #[error("unsupported repository state: {0}")]
    UnsupportedRepositoryState(String),

    #[error("snapshot verification failed: {0}")]
    Verification(String),
}

pub type Result<T> = std::result::Result<T, DevRelayError>;
