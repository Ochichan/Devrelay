//! Background checkpoint orchestration.
//!
//! Filesystem watcher output is only a hint. The manager below is fed by the
//! debouncer after a quiet window, then re-reads Git state before deciding
//! whether to create durable snapshot metadata.

use crate::{
    AnchorSnapshotRepo, DebouncedCheckpoint, DevRelayError, DevRelayHome, GitRepo, Manifest,
    ProjectRegistryEntry, ProtectionStatus, ProtectionStatusEvent, Result,
    SnapshotCheckpointResult, SnapshotStore, StoredSnapshot, WorkspaceRegistryEntry,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const DEFAULT_BACKGROUND_FAILURE_NOTIFICATION_THRESHOLD: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundWorkspace {
    pub project_id: String,
    pub workspace_id: String,
    pub device_id: Option<String>,
    pub repo_path: PathBuf,
    pub manifest_path: Option<PathBuf>,
}

impl BackgroundWorkspace {
    pub fn from_registry(
        project: &ProjectRegistryEntry,
        workspace: &WorkspaceRegistryEntry,
    ) -> Self {
        Self {
            project_id: project.project_id.clone(),
            workspace_id: workspace.workspace_id.clone(),
            device_id: Some(workspace.device_id.clone()),
            repo_path: workspace.local_path.clone(),
            manifest_path: project
                .manifest_path
                .clone()
                .or_else(|| Some(project.local_path.join("devrelay.toml"))),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCheckpointState {
    pub dirty: bool,
    pub last_observed_generation: Option<u64>,
    pub last_checkpoint_generation: Option<u64>,
    pub last_snapshot_id: Option<String>,
    pub last_state_hash: Option<String>,
    pub repeated_failures: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum BackgroundCheckpointOutcome {
    Created {
        snapshot_id: String,
        state_hash: String,
    },
    Unchanged {
        snapshot_id: String,
        state_hash: String,
    },
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundCheckpointReport {
    pub project_id: String,
    pub workspace_id: String,
    pub source_generation: u64,
    pub outcome: BackgroundCheckpointOutcome,
    pub snapshot: Option<StoredSnapshot>,
    pub event: ProtectionStatusEvent,
}

#[derive(Debug, Clone)]
pub struct BackgroundCheckpointManager {
    failure_notification_threshold: u32,
    workspaces: BTreeMap<String, WorkspaceCheckpointState>,
}

impl Default for BackgroundCheckpointManager {
    fn default() -> Self {
        Self::new()
    }
}

impl BackgroundCheckpointManager {
    pub fn new() -> Self {
        Self::with_failure_notification_threshold(DEFAULT_BACKGROUND_FAILURE_NOTIFICATION_THRESHOLD)
    }

    pub fn with_failure_notification_threshold(failure_notification_threshold: u32) -> Self {
        Self {
            failure_notification_threshold: failure_notification_threshold.max(1),
            workspaces: BTreeMap::new(),
        }
    }

    pub fn workspace_state(&self, workspace_id: &str) -> Option<&WorkspaceCheckpointState> {
        self.workspaces.get(workspace_id)
    }

    pub fn mark_dirty(
        &mut self,
        workspace: &BackgroundWorkspace,
        source_generation: u64,
    ) -> ProtectionStatusEvent {
        let state = self
            .workspaces
            .entry(workspace.workspace_id.clone())
            .or_default();
        state.dirty = true;
        state.last_observed_generation = Some(
            state
                .last_observed_generation
                .unwrap_or_default()
                .max(source_generation),
        );
        protection_status_event(
            workspace,
            ProtectionStatus::Dirty,
            source_generation,
            None,
            None,
            state.repeated_failures,
            false,
            None,
        )
    }

    pub fn handle_checkpoint(
        &mut self,
        home: &DevRelayHome,
        workspace: &BackgroundWorkspace,
        checkpoint: &DebouncedCheckpoint,
    ) -> BackgroundCheckpointReport {
        self.handle_checkpoint_with_anchor(home, workspace, checkpoint, None)
    }

    pub fn handle_checkpoint_with_anchor(
        &mut self,
        home: &DevRelayHome,
        workspace: &BackgroundWorkspace,
        checkpoint: &DebouncedCheckpoint,
        anchor_repo: Option<&AnchorSnapshotRepo>,
    ) -> BackgroundCheckpointReport {
        let result = self.handle_checkpoint_inner(home, workspace, checkpoint, anchor_repo);
        match result {
            Ok(SnapshotCheckpointResult::Created { snapshot }) => {
                self.record_created(workspace, checkpoint.source_generation, *snapshot)
            }
            Ok(SnapshotCheckpointResult::Unchanged {
                snapshot_id,
                state_hash,
            }) => self.record_unchanged(
                workspace,
                checkpoint.source_generation,
                snapshot_id,
                state_hash,
            ),
            Err(err) => self.record_failure(workspace, checkpoint.source_generation, err),
        }
    }

    fn handle_checkpoint_inner(
        &self,
        home: &DevRelayHome,
        workspace: &BackgroundWorkspace,
        checkpoint: &DebouncedCheckpoint,
        anchor_repo: Option<&AnchorSnapshotRepo>,
    ) -> Result<SnapshotCheckpointResult> {
        if checkpoint.workspace_id != workspace.workspace_id {
            return Err(DevRelayError::Config(format!(
                "debounced checkpoint for workspace {} does not match registered workspace {}",
                checkpoint.workspace_id, workspace.workspace_id
            )));
        }

        let repo = GitRepo::new(&workspace.repo_path);
        let manifest_path = workspace
            .manifest_path
            .clone()
            .unwrap_or_else(|| repo.path().join("devrelay.toml"));
        let manifest = Manifest::load(&manifest_path).map_err(|err| {
            DevRelayError::Manifest(format!("failed to load {}: {err}", manifest_path.display()))
        })?;
        if manifest.project_id != workspace.project_id {
            return Err(DevRelayError::Config(format!(
                "manifest project_id {} does not match registered project {}",
                manifest.project_id, workspace.project_id
            )));
        }

        let _status = repo.status()?;
        let mut store = SnapshotStore::open(home, &manifest.project_id)?;
        let result = store.checkpoint_if_changed(
            &repo,
            &manifest,
            false,
            Some(format!("background-{}", checkpoint.source_generation)),
        )?;
        if let Some(anchor_repo) = anchor_repo
            && let SnapshotCheckpointResult::Created { snapshot } = &result
        {
            anchor_repo.import_snapshot_from_store(&store, &snapshot.snapshot_id)?;
        }
        Ok(result)
    }

    fn record_created(
        &mut self,
        workspace: &BackgroundWorkspace,
        source_generation: u64,
        snapshot: StoredSnapshot,
    ) -> BackgroundCheckpointReport {
        let snapshot_id = snapshot.snapshot_id.clone();
        let state_hash = snapshot.metadata.state_hash.clone();
        self.record_success(workspace, source_generation, &snapshot_id, &state_hash);
        let event = protection_status_event(
            workspace,
            ProtectionStatus::Protected,
            source_generation,
            Some(snapshot_id.clone()),
            Some(state_hash.clone()),
            0,
            false,
            None,
        );
        BackgroundCheckpointReport {
            project_id: workspace.project_id.clone(),
            workspace_id: workspace.workspace_id.clone(),
            source_generation,
            outcome: BackgroundCheckpointOutcome::Created {
                snapshot_id,
                state_hash,
            },
            snapshot: Some(snapshot),
            event,
        }
    }

    fn record_unchanged(
        &mut self,
        workspace: &BackgroundWorkspace,
        source_generation: u64,
        snapshot_id: String,
        state_hash: String,
    ) -> BackgroundCheckpointReport {
        self.record_success(workspace, source_generation, &snapshot_id, &state_hash);
        let event = protection_status_event(
            workspace,
            ProtectionStatus::Unchanged,
            source_generation,
            Some(snapshot_id.clone()),
            Some(state_hash.clone()),
            0,
            false,
            None,
        );
        BackgroundCheckpointReport {
            project_id: workspace.project_id.clone(),
            workspace_id: workspace.workspace_id.clone(),
            source_generation,
            outcome: BackgroundCheckpointOutcome::Unchanged {
                snapshot_id,
                state_hash,
            },
            snapshot: None,
            event,
        }
    }

    fn record_success(
        &mut self,
        workspace: &BackgroundWorkspace,
        source_generation: u64,
        snapshot_id: &str,
        state_hash: &str,
    ) {
        let state = self
            .workspaces
            .entry(workspace.workspace_id.clone())
            .or_default();
        state.dirty = false;
        state.last_observed_generation = Some(
            state
                .last_observed_generation
                .unwrap_or_default()
                .max(source_generation),
        );
        state.last_checkpoint_generation = Some(source_generation);
        state.last_snapshot_id = Some(snapshot_id.to_string());
        state.last_state_hash = Some(state_hash.to_string());
        state.repeated_failures = 0;
    }

    fn record_failure(
        &mut self,
        workspace: &BackgroundWorkspace,
        source_generation: u64,
        err: DevRelayError,
    ) -> BackgroundCheckpointReport {
        let repeated_failures = {
            let state = self
                .workspaces
                .entry(workspace.workspace_id.clone())
                .or_default();
            state.dirty = true;
            state.last_observed_generation = Some(
                state
                    .last_observed_generation
                    .unwrap_or_default()
                    .max(source_generation),
            );
            state.repeated_failures = state.repeated_failures.saturating_add(1);
            state.repeated_failures
        };
        let user_visible = repeated_failures >= self.failure_notification_threshold;
        let detail = err.to_string();
        let event = protection_status_event(
            workspace,
            ProtectionStatus::Failed,
            source_generation,
            None,
            None,
            repeated_failures,
            user_visible,
            Some(detail.clone()),
        );
        BackgroundCheckpointReport {
            project_id: workspace.project_id.clone(),
            workspace_id: workspace.workspace_id.clone(),
            source_generation,
            outcome: BackgroundCheckpointOutcome::Failed { error: detail },
            snapshot: None,
            event,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn protection_status_event(
    workspace: &BackgroundWorkspace,
    status: ProtectionStatus,
    source_generation: u64,
    snapshot_id: Option<String>,
    state_hash: Option<String>,
    repeated_failures: u32,
    user_visible: bool,
    detail: Option<String>,
) -> ProtectionStatusEvent {
    ProtectionStatusEvent {
        project_id: workspace.project_id.clone(),
        workspace_id: workspace.workspace_id.clone(),
        device_id: workspace.device_id.clone(),
        status,
        source_generation,
        snapshot_id,
        state_hash,
        repeated_failures,
        user_visible,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AdaptiveDebouncer, BackgroundDebouncePolicy, DebounceFlushReason, FilesystemEventKind,
        FilesystemRawEvent, FilesystemWatchState, WorkspaceWatch,
    };
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    fn manifest_text() -> &'static str {
        r#"
schema = 1
project_id = "bg-project"
name = "Background Project"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#
    }

    fn init_repo(path: &Path) -> GitRepo {
        fs::create_dir(path).unwrap();
        let repo = GitRepo::new(path);
        repo.run(&["init", "-b", "main"]).unwrap();
        repo.run(&["config", "user.name", "DevRelay Test"]).unwrap();
        repo.run(&["config", "user.email", "devrelay-test@example.local"])
            .unwrap();
        fs::write(path.join("devrelay.toml"), manifest_text()).unwrap();
        fs::write(path.join("tracked.txt"), "base\n").unwrap();
        repo.run(&["add", "."]).unwrap();
        repo.run(&["commit", "-m", "base"]).unwrap();
        repo
    }

    fn background_workspace(path: &Path) -> BackgroundWorkspace {
        BackgroundWorkspace {
            project_id: "bg-project".to_string(),
            workspace_id: "w-source".to_string(),
            device_id: Some("device-a".to_string()),
            repo_path: path.to_path_buf(),
            manifest_path: Some(path.join("devrelay.toml")),
        }
    }

    fn debounced_checkpoint(source_generation: u64) -> DebouncedCheckpoint {
        DebouncedCheckpoint {
            workspace_id: "w-source".to_string(),
            source_generation,
            reason: DebounceFlushReason::QuietWindow,
            paths: vec![PathBuf::from("tracked.txt")],
        }
    }

    #[test]
    fn background_checkpoint_creates_local_snapshot_after_quiet_window() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        init_repo(&source_path);
        let workspace = background_workspace(&source_path);
        let mut watch_state = FilesystemWatchState::default();
        watch_state
            .register_workspace(WorkspaceWatch::new("w-source", &source_path).unwrap())
            .unwrap();
        let mut debouncer = AdaptiveDebouncer::new(BackgroundDebouncePolicy {
            first_event_quiet: Duration::from_secs(1),
            min_checkpoint_interval: Duration::ZERO,
            max_dirty_interval: Duration::from_secs(60),
            publish_quiet: Duration::from_secs(10),
            max_publish_interval: Duration::from_secs(60),
        });
        let mut manager = BackgroundCheckpointManager::new();

        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        watch_state.observe_raw_event(&FilesystemRawEvent::new(
            FilesystemEventKind::Modify,
            [source_path.join("tracked.txt")],
        ));
        let changes = watch_state.drain_coalesced();
        assert_eq!(changes.len(), 1);
        for change in changes {
            let dirty = manager.mark_dirty(&workspace, change.source_generation);
            assert_eq!(dirty.status, ProtectionStatus::Dirty);
            assert!(!dirty.user_visible);
            debouncer.observe_change(change, Duration::ZERO);
        }

        let drain = debouncer.drain_due(Duration::from_secs(2));
        assert_eq!(drain.checkpoints.len(), 1);
        let report = manager.handle_checkpoint(&home, &workspace, &drain.checkpoints[0]);

        assert_eq!(report.event.status, ProtectionStatus::Protected);
        assert!(!report.event.user_visible);
        let BackgroundCheckpointOutcome::Created {
            snapshot_id,
            state_hash,
        } = &report.outcome
        else {
            panic!("expected created checkpoint");
        };
        let stored = report.snapshot.as_ref().unwrap();
        assert_eq!(snapshot_id, &stored.snapshot_id);
        assert_eq!(state_hash, &stored.metadata.state_hash);
        let state = manager.workspace_state("w-source").unwrap();
        assert!(!state.dirty);
        assert_eq!(state.repeated_failures, 0);

        let store = SnapshotStore::open(&home, "bg-project").unwrap();
        assert_eq!(store.list_snapshots().unwrap().len(), 1);
    }

    #[test]
    fn background_checkpoint_publishes_created_snapshot_to_anchor_when_available() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        init_repo(&source_path);
        let workspace = background_workspace(&source_path);
        let anchor = AnchorSnapshotRepo::open(&home, "bg-project").unwrap();
        let mut manager = BackgroundCheckpointManager::new();

        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let report = manager.handle_checkpoint_with_anchor(
            &home,
            &workspace,
            &debounced_checkpoint(1),
            Some(&anchor),
        );

        assert_eq!(report.event.status, ProtectionStatus::Protected);
        let stored = report.snapshot.as_ref().unwrap();
        anchor.verify_snapshot_available(&stored.metadata).unwrap();
        assert!(
            GitRepo::new(anchor.repo_path())
                .run(&["rev-parse", "--verify", &stored.metadata.work_ref()])
                .is_ok()
        );
    }

    #[test]
    fn background_checkpoint_skips_semantic_state_that_is_already_protected() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        init_repo(&source_path);
        let workspace = background_workspace(&source_path);
        let mut manager = BackgroundCheckpointManager::new();

        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let first = manager.handle_checkpoint(&home, &workspace, &debounced_checkpoint(1));
        let stored = first.snapshot.as_ref().unwrap();
        let first_snapshot_id = stored.snapshot_id.clone();
        let first_state_hash = stored.metadata.state_hash.clone();

        let second = manager.handle_checkpoint(&home, &workspace, &debounced_checkpoint(2));

        assert_eq!(
            second.outcome,
            BackgroundCheckpointOutcome::Unchanged {
                snapshot_id: first_snapshot_id.clone(),
                state_hash: first_state_hash.clone(),
            }
        );
        assert_eq!(second.event.status, ProtectionStatus::Unchanged);
        assert_eq!(
            second.event.snapshot_id.as_deref(),
            Some(first_snapshot_id.as_str())
        );
        assert!(!second.event.user_visible);
        let store = SnapshotStore::open(&home, "bg-project").unwrap();
        assert_eq!(store.list_snapshots().unwrap().len(), 1);
    }

    #[test]
    fn background_checkpoint_surfaces_repeated_failures() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        init_repo(&source_path);
        let mut workspace = background_workspace(&source_path);
        workspace.manifest_path = Some(source_path.join("missing-devrelay.toml"));
        let mut manager = BackgroundCheckpointManager::new();

        let first = manager.handle_checkpoint(&home, &workspace, &debounced_checkpoint(1));
        let second = manager.handle_checkpoint(&home, &workspace, &debounced_checkpoint(2));
        let third = manager.handle_checkpoint(&home, &workspace, &debounced_checkpoint(3));

        assert!(matches!(
            first.outcome,
            BackgroundCheckpointOutcome::Failed { .. }
        ));
        assert!(!first.event.user_visible);
        assert!(!second.event.user_visible);
        assert!(third.event.user_visible);
        assert_eq!(third.event.status, ProtectionStatus::Failed);
        assert_eq!(third.event.repeated_failures, 3);
        assert!(
            third
                .event
                .detail
                .as_deref()
                .unwrap()
                .contains("missing-devrelay.toml")
        );
        let state = manager.workspace_state("w-source").unwrap();
        assert!(state.dirty);
        assert_eq!(state.repeated_failures, 3);
    }
}
