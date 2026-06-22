//! Core DevRelay state capture and apply primitives.
//!
//! The public M0 surface is intentionally small: manifest parsing, Git-backed
//! status collection, untracked-path classification, and local snapshot
//! creation/apply helpers. Higher-level registry, agent, and UI state should be
//! built above this crate rather than inferred independently.
//!
//! Parse a manifest:
//!
//! ```
//! use devrelay_core::{Manifest, UntrackedPolicy};
//!
//! let manifest = Manifest::parse(r#"
//! schema = 1
//! project_id = "12345678"
//! name = "demo"
//!
//! [workspace]
//! untracked = "safe"
//! portable_paths = "strict"
//! "#)?;
//!
//! assert_eq!(manifest.workspace.untracked, UntrackedPolicy::Safe);
//! # Ok::<(), devrelay_core::DevRelayError>(())
//! ```
//!
//! Collect Git status:
//!
//! ```no_run
//! use devrelay_core::GitRepo;
//!
//! let repo = GitRepo::new(".");
//! let status = repo.status()?;
//! println!("{}", status.short_summary());
//! # Ok::<(), devrelay_core::DevRelayError>(())
//! ```

mod error;
mod git;
pub mod manifest;
mod policy;
mod snapshot;
mod snapshot_schema;

pub use error::{DevRelayError, Result};
pub use git::{GitRepo, GitStatus, StatusCounts, StatusEntry, StatusEntryKind};
pub use manifest::{
    DirtyTargetPolicy, EnvironmentConfig, EnvironmentKind, EnvironmentProfile, HandoffConfig,
    Manifest, PatternConfig, PortablePathsPolicy, RestoreTerminals, SecretConfig, SecretMode,
    SyncConfig, SyncMode, TaskCacheMode, TaskConfig, TaskSandbox, UntrackedPolicy, WorkspaceConfig,
};
pub use policy::{ClassifiedPath, PathDecision, classification_reason, classify_untracked_paths};
pub use snapshot::{
    ApplyPlan, VerificationDetails, apply_snapshot, create_snapshot, plan_apply_snapshot,
    read_snapshot_file, verify_snapshot, write_snapshot_file,
};
pub use snapshot_schema::SnapshotMetadata;
