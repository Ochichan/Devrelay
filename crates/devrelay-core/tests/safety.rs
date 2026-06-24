use devrelay_core::{
    AuditOutcome, BackgroundCheckpointManager, BackgroundCheckpointOutcome, BackgroundWorkspace,
    CanonicalPublishRequest, CodeChangingTaskTestCommand, CommandTrustDecision, CommandTrustStatus,
    DebounceFlushReason, DebouncedCheckpoint, DevRelayError, DevRelayHome, EnvironmentKind,
    EnvironmentSelectionContext, GitRepo, HandoffJournalPhase, HandoffState, LeaseRecord,
    LeaseState, Manifest, MetadataDb, ProtectionStatus, SESSION_ID_PREFIX, SnapshotMetadata,
    SnapshotStore, TaskRunnerEnvironmentState, TaskRunnerSecretState, TaskRunnerSidecarState,
    TaskRunnerWorkspace, TaskRunnerWorkspaceRetentionPolicy, VerificationDetails, apply_snapshot,
    classification_reason, create_snapshot, environment_profile_command_scope,
    plan_code_changing_task, select_environment_profile,
};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

const MANIFEST_TEXT: &str = r#"
schema = 1
project_id = "12345678"
name = "safety"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#;

fn manifest() -> Manifest {
    Manifest::parse(MANIFEST_TEXT).unwrap()
}

fn init_repo(path: &Path) -> GitRepo {
    fs::create_dir_all(path).unwrap();
    let repo = GitRepo::new(path);
    repo.run(&["init", "-b", "main"]).unwrap();
    repo.run(&["config", "user.name", "DevRelay Test"]).unwrap();
    repo.run(&["config", "user.email", "devrelay-test@example.local"])
        .unwrap();
    repo
}

fn commit_base(repo: &GitRepo, path: &Path) {
    fs::write(path.join("README.md"), "base\n").unwrap();
    repo.run(&["add", "README.md"]).unwrap();
    repo.run(&["commit", "-m", "base"]).unwrap();
}

fn clone_repo(source: &GitRepo, source_path: &Path, target_path: &Path) -> GitRepo {
    source
        .run_with_env(
            [
                OsString::from("clone"),
                source_path.as_os_str().to_os_string(),
                target_path.as_os_str().to_os_string(),
            ],
            &[],
        )
        .unwrap();
    GitRepo::new(target_path)
}

fn anchor_db() -> (tempfile::TempDir, MetadataDb, String, LeaseRecord) {
    let temp = tempfile::tempdir().unwrap();
    let db = MetadataDb::open(temp.path().join("metadata.sqlite")).unwrap();
    let session = db
        .ensure_default_session("project123", "Safety Project", None)
        .unwrap();
    let lease = LeaseRecord {
        lease_id: "lease-1".to_string(),
        project_id: "project123".to_string(),
        session_id: session.session_id.clone(),
        state: LeaseState::Active,
        epoch: 2,
        holder_device_id: Some("device-a".to_string()),
        latest_snapshot_id: None,
        handoff_id: None,
    };
    db.upsert_lease(&lease).unwrap();
    (temp, db, session.session_id, lease)
}

fn publish_metadata(snapshot_id: &str, session_id: &str) -> SnapshotMetadata {
    let mut metadata: SnapshotMetadata =
        serde_json::from_str(include_str!("fixtures/snapshot_metadata_v1.json")).unwrap();
    metadata.project_id = "project123".to_string();
    metadata.project_name = "Safety Project".to_string();
    metadata.session_id = Some(session_id.to_string());
    metadata.snapshot_id = snapshot_id.to_string();
    metadata.parent_snapshot_id = None;
    metadata
}

mod no_silent_overwrite {
    //! Invariant: `safety/no_silent_overwrite`; see `docs/data-loss-safety.md`.

    use super::*;

    #[test]
    fn dirty_target_apply_is_rejected_and_target_bytes_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let target_path = temp.path().join("target");
        let source = init_repo(&source_path);
        commit_base(&source, &source_path);

        fs::write(source_path.join("README.md"), "source change\n").unwrap();
        let snapshot = create_snapshot(&source, &manifest()).unwrap();
        let target = clone_repo(&source, &source_path, &target_path);
        fs::write(target_path.join("README.md"), "target-only change\n").unwrap();
        fs::write(target_path.join("local-notes.md"), "target local\n").unwrap();

        let err = apply_snapshot(&target, &source, &snapshot).unwrap_err();

        assert!(matches!(err, DevRelayError::TargetDirty(_)));
        assert_eq!(
            fs::read_to_string(target_path.join("README.md")).unwrap(),
            "target-only change\n"
        );
        assert_eq!(
            fs::read_to_string(target_path.join("local-notes.md")).unwrap(),
            "target local\n"
        );
    }
}

mod no_unverified_handoff {
    //! Invariant: `safety/no_unverified_handoff`; see `docs/data-loss-safety.md`.

    use super::*;

    #[test]
    fn lease_cannot_transfer_before_target_verification_and_source_ready() {
        let (_temp, mut db, _session_id, lease) = anchor_db();
        let handoff = db
            .begin_handoff(&lease.lease_id, "device-a", "device-b", "gen-1", 600)
            .unwrap();

        let err = db
            .commit_handoff(
                &handoff.handoff_id,
                "gen-1",
                handoff.expires_at_unix_seconds.saturating_sub(1),
            )
            .unwrap_err();

        assert!(err.to_string().contains("handoff is not source-ready"));
        let unchanged = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(unchanged.holder_device_id.as_deref(), Some("device-a"));
        assert_eq!(unchanged.epoch, 2);
        assert_eq!(unchanged.state, LeaseState::HandoffPending);
        assert_eq!(
            unchanged.handoff_id.as_deref(),
            Some(handoff.handoff_id.as_str())
        );
        assert_eq!(
            db.get_handoff(&handoff.handoff_id).unwrap().unwrap().state,
            HandoffState::TargetPrepare
        );
        assert!(
            !db.list_handoff_journal(&handoff.handoff_id)
                .unwrap()
                .iter()
                .any(|entry| entry.phase == HandoffJournalPhase::LeaseCommitted)
        );
    }
}

mod stale_publish_is_fork {
    //! Invariant: `safety/stale_publish_is_fork`; see `docs/data-loss-safety.md`.

    use super::*;

    #[test]
    fn stale_publish_is_stored_without_advancing_canonical_latest() {
        let (_temp, mut db, session_id, lease) = anchor_db();
        let canonical = publish_metadata("s1_000000000000000000000201", &session_id);
        db.publish_snapshot_canonical(CanonicalPublishRequest {
            lease_id: &lease.lease_id,
            session_id: &session_id,
            expected_epoch: 2,
            holder_device_id: "device-a",
            expected_latest_snapshot_id: None,
            metadata: &canonical,
            pinned: false,
            label: Some("canonical"),
        })
        .unwrap();

        let mut advanced = db.get_lease(&lease.lease_id).unwrap().unwrap();
        advanced.epoch = 3;
        db.upsert_lease(&advanced).unwrap();

        let stale = publish_metadata("s1_000000000000000000000202", &session_id);
        let err = db
            .publish_snapshot_canonical(CanonicalPublishRequest {
                lease_id: &lease.lease_id,
                session_id: &session_id,
                expected_epoch: 2,
                holder_device_id: "device-a",
                expected_latest_snapshot_id: Some(&canonical.snapshot_id),
                metadata: &stale,
                pinned: true,
                label: Some("stale"),
            })
            .unwrap_err();

        assert!(err.to_string().contains("stale publish"));
        let stored = db.list_stored_snapshots(Some("project123")).unwrap();
        assert_eq!(
            stored
                .iter()
                .map(|snapshot| snapshot.snapshot_id.as_str())
                .collect::<Vec<_>>(),
            vec![canonical.snapshot_id.as_str(), stale.snapshot_id.as_str()]
        );
        assert_eq!(
            db.get_lease(&lease.lease_id)
                .unwrap()
                .unwrap()
                .latest_snapshot_id
                .as_deref(),
            Some(canonical.snapshot_id.as_str())
        );
        let blocked = db.list_audit_events(Some("project123"), 1).unwrap();
        assert_eq!(blocked[0].outcome, AuditOutcome::Blocked);
        assert_eq!(
            blocked[0].snapshot_id.as_deref(),
            Some(stale.snapshot_id.as_str())
        );
    }
}

mod no_plaintext_secret_snapshot {
    //! Invariant: `safety/no_plaintext_secret_snapshot`; see `docs/data-loss-safety.md`.

    use super::*;

    #[test]
    fn plaintext_secret_files_are_excluded_from_snapshot_work_tree() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        commit_base(&source, &source_path);
        fs::write(source_path.join("notes.md"), "carry me\n").unwrap();
        fs::write(source_path.join(".env"), "DATABASE_URL=secret\n").unwrap();
        fs::write(
            source_path.join("private.pem"),
            "-----BEGIN PRIVATE KEY-----\nsecret\n",
        )
        .unwrap();

        let snapshot = create_snapshot(&source, &manifest()).unwrap();
        let work_tree_paths = source
            .run(&["ls-tree", "-r", "--name-only", &snapshot.work_tree_oid])
            .unwrap();

        assert!(
            snapshot
                .included_untracked
                .contains(&"notes.md".to_string())
        );
        assert!(!snapshot.included_untracked.contains(&".env".to_string()));
        assert!(snapshot.excluded.iter().any(|item| {
            item.path == ".env" && item.reason == classification_reason::SECRET_FILENAME
        }));
        assert!(snapshot.excluded.iter().any(|item| {
            item.path == "private.pem" && item.reason == classification_reason::PRIVATE_KEY_FILENAME
        }));
        assert!(!work_tree_paths.lines().any(|path| path == ".env"));
        assert!(!work_tree_paths.lines().any(|path| path == "private.pem"));
    }
}

mod no_active_workspace_remote_task {
    //! Invariant: `safety/no_active_workspace_remote_task`; see `docs/data-loss-safety.md`.

    use super::*;

    #[test]
    fn code_changing_task_requires_noncanonical_runner_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let canonical_path = temp.path().join("canonical");
        let runner_path = temp.path().join("runner");
        let canonical_repo = init_repo(&canonical_path);
        commit_base(&canonical_repo, &canonical_path);
        fs::create_dir_all(&runner_path).unwrap();
        fs::write(runner_path.join("README.md"), "runner-only change\n").unwrap();
        let canonical_before = fs::read_to_string(canonical_path.join("README.md")).unwrap();

        let canonical_workspace = task_workspace(&canonical_path, true);
        let err = plan_code_changing_task(&canonical_workspace, None, Vec::new()).unwrap_err();

        assert!(err.to_string().contains("canonical session"));
        assert_eq!(
            fs::read_to_string(canonical_path.join("README.md")).unwrap(),
            canonical_before
        );

        let runner_workspace = task_workspace(&runner_path, false);
        let plan = plan_code_changing_task(
            &runner_workspace,
            Some("se_parent".to_string()),
            vec![CodeChangingTaskTestCommand {
                name: "unit".to_string(),
                command: vec!["cargo".to_string(), "test".to_string()],
                timeout_seconds: Some(60),
            }],
        )
        .unwrap();

        assert!(plan.code_changing);
        assert!(plan.task_session_id.starts_with(SESSION_ID_PREFIX));
        assert_eq!(plan.parent_session_id.as_deref(), Some("se_parent"));
        assert_eq!(plan.isolated_workspace, runner_path);
        assert_ne!(plan.isolated_workspace, canonical_path);
        assert!(!plan.auto_merge);
        assert!(
            plan.explanation
                .iter()
                .any(|line| line.contains("non-canonical"))
        );
        assert_eq!(
            fs::read_to_string(canonical_path.join("README.md")).unwrap(),
            canonical_before
        );
    }

    fn task_workspace(path: &Path, canonical_session: bool) -> TaskRunnerWorkspace {
        TaskRunnerWorkspace {
            task_run_id: "tr_safety".to_string(),
            project_id: "12345678".to_string(),
            task_name: "agent-edit".to_string(),
            path: path.to_path_buf(),
            snapshot_id: "s1_0123456789abcdef01234567".to_string(),
            canonical_session,
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
                head_oid: "head".to_string(),
                index_tree_oid: "index".to_string(),
                work_tree_oid: "work".to_string(),
                state_hash: "state".to_string(),
                included_untracked: Vec::new(),
                excluded_paths: Vec::new(),
            },
            retention_policy: TaskRunnerWorkspaceRetentionPolicy::delete_on_cleanup(),
        }
    }
}

mod no_untrusted_remote_execution {
    //! Invariant: `safety/no_untrusted_remote_execution`; see `docs/data-loss-safety.md`.

    use super::*;

    #[test]
    fn command_hash_requires_project_device_scoped_approval() {
        let temp = tempfile::tempdir().unwrap();
        let mut db = MetadataDb::open(temp.path().join("metadata.sqlite")).unwrap();
        db.ensure_default_session("project-a", "Safety Project", None)
            .unwrap();
        let scope = environment_profile_command_scope("remote");
        let hash_a = "a".repeat(64);
        let hash_b = "b".repeat(64);
        let hash_c = "c".repeat(64);

        let unapproved = db
            .evaluate_command_trust("project-a", "device-a", &scope, &hash_a)
            .unwrap();
        assert_eq!(unapproved.status, CommandTrustStatus::Unapproved);
        assert!(!unapproved.status.approved());

        db.record_command_trust_decision_at(
            "project-a",
            "device-a",
            &scope,
            &hash_a,
            CommandTrustDecision::TrustThisVersion,
            10,
        )
        .unwrap();
        let trusted = db
            .evaluate_command_trust("project-a", "device-a", &scope, &hash_a)
            .unwrap();
        assert_eq!(trusted.status, CommandTrustStatus::Trusted);
        assert!(trusted.status.approved());

        let changed = db
            .evaluate_command_trust("project-a", "device-a", &scope, &hash_b)
            .unwrap();
        assert_eq!(changed.status, CommandTrustStatus::Changed);
        assert_eq!(changed.previous_hash.as_deref(), Some(hash_a.as_str()));
        assert!(!changed.status.approved());

        let other_device = db
            .evaluate_command_trust("project-a", "device-b", &scope, &hash_a)
            .unwrap();
        assert_eq!(other_device.status, CommandTrustStatus::Unapproved);
        assert!(!other_device.status.approved());

        db.record_command_trust_decision_at(
            "project-a",
            "device-a",
            &scope,
            &hash_b,
            CommandTrustDecision::Reject,
            11,
        )
        .unwrap();
        let rejected = db
            .evaluate_command_trust("project-a", "device-a", &scope, &hash_b)
            .unwrap();
        assert_eq!(rejected.status, CommandTrustStatus::Rejected);
        assert!(!rejected.status.approved());

        db.record_command_trust_decision_at(
            "project-a",
            "device-a",
            &scope,
            &hash_c,
            CommandTrustDecision::AllowOnce,
            12,
        )
        .unwrap();
        let before = db
            .evaluate_command_trust("project-a", "device-a", &scope, &hash_c)
            .unwrap();
        assert_eq!(before.status, CommandTrustStatus::ApprovedOnce);
        assert!(before.status.approved());

        db.authorize_command_trust_at("project-a", "device-a", &scope, &hash_c, 13)
            .unwrap();
        let consumed = db
            .evaluate_command_trust("project-a", "device-a", &scope, &hash_c)
            .unwrap();
        assert_eq!(consumed.status, CommandTrustStatus::Unapproved);
        assert!(!consumed.status.approved());
    }

    #[test]
    fn script_profile_is_not_selected_without_trusted_command_scope() {
        let manifest = Manifest::parse(
            r#"
schema = 1
project_id = "12345678"
name = "remote-command"

[workspace]
untracked = "safe"
portable_paths = "strict"

[environment.profiles.remote]
kind = "script"
targets = ["darwin-*", "linux-*"]
command = ["sh", "-lc", "./bootstrap.sh"]
"#,
        )
        .unwrap();
        let scope = environment_profile_command_scope("remote");
        let untrusted_context = EnvironmentSelectionContext::with_platform_key("darwin-arm64")
            .with_available_kind(EnvironmentKind::Script);

        let blocked = select_environment_profile(&manifest, &untrusted_context);

        assert_eq!(blocked.profile_name, None);
        assert_eq!(blocked.command_scope, None);
        assert!(
            blocked
                .explanation
                .iter()
                .any(|line| line.contains("not trusted"))
        );

        let trusted_context = untrusted_context.with_trusted_command_scope(scope.clone());
        let selected = select_environment_profile(&manifest, &trusted_context);

        assert_eq!(selected.profile_name.as_deref(), Some("remote"));
        assert_eq!(selected.command_scope.as_deref(), Some(scope.as_str()));
    }
}

mod no_background_auto_merge {
    //! Invariant: `safety/no_background_auto_merge`; see `docs/data-loss-safety.md`.

    use super::*;

    fn background_workspace(path: &Path) -> BackgroundWorkspace {
        BackgroundWorkspace {
            project_id: "12345678".to_string(),
            workspace_id: "w-source".to_string(),
            device_id: Some("device-a".to_string()),
            repo_path: path.to_path_buf(),
            manifest_path: Some(path.join("devrelay.toml")),
        }
    }

    #[test]
    fn background_checkpoint_preserves_worktree_and_index_status() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        commit_base(&source, &source_path);
        fs::write(source_path.join("devrelay.toml"), MANIFEST_TEXT).unwrap();
        source.run(&["add", "devrelay.toml"]).unwrap();
        source.run(&["commit", "-m", "manifest"]).unwrap();
        fs::write(source_path.join("README.md"), "unstaged change\n").unwrap();
        fs::write(source_path.join("staged.txt"), "staged\n").unwrap();
        source.run(&["add", "staged.txt"]).unwrap();
        fs::write(source_path.join("notes.md"), "untracked\n").unwrap();
        let before = source.run(&["status", "--porcelain=v2", "-z"]).unwrap();
        let workspace = background_workspace(&source_path);
        let checkpoint = DebouncedCheckpoint {
            workspace_id: "w-source".to_string(),
            source_generation: 1,
            reason: DebounceFlushReason::QuietWindow,
            paths: vec![
                PathBuf::from("README.md"),
                PathBuf::from("staged.txt"),
                PathBuf::from("notes.md"),
            ],
        };
        let mut manager = BackgroundCheckpointManager::new();

        let report = manager.handle_checkpoint(&home, &workspace, &checkpoint);

        assert!(matches!(
            report.outcome,
            BackgroundCheckpointOutcome::Created { .. }
        ));
        let after = source.run(&["status", "--porcelain=v2", "-z"]).unwrap();
        assert_eq!(after, before);
    }
}

mod watcher_events_are_hints {
    //! Invariant: `safety/watcher_events_are_hints`; see `docs/data-loss-safety.md`.

    use super::*;

    fn background_workspace(path: &Path) -> BackgroundWorkspace {
        BackgroundWorkspace {
            project_id: "12345678".to_string(),
            workspace_id: "w-source".to_string(),
            device_id: Some("device-a".to_string()),
            repo_path: path.to_path_buf(),
            manifest_path: Some(path.join("devrelay.toml")),
        }
    }

    fn debounced_checkpoint(source_generation: u64, paths: Vec<PathBuf>) -> DebouncedCheckpoint {
        DebouncedCheckpoint {
            workspace_id: "w-source".to_string(),
            source_generation,
            reason: DebounceFlushReason::QuietWindow,
            paths,
        }
    }

    #[test]
    fn phantom_watcher_path_does_not_create_new_snapshot_after_clean_rescan() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        commit_base(&source, &source_path);
        fs::write(source_path.join("devrelay.toml"), MANIFEST_TEXT).unwrap();
        source.run(&["add", "devrelay.toml"]).unwrap();
        source.run(&["commit", "-m", "manifest"]).unwrap();
        let workspace = background_workspace(&source_path);
        let mut manager = BackgroundCheckpointManager::new();

        fs::write(source_path.join("README.md"), "actual change\n").unwrap();
        let first = manager.handle_checkpoint(
            &home,
            &workspace,
            &debounced_checkpoint(1, vec![PathBuf::from("README.md")]),
        );
        let stored = first.snapshot.as_ref().unwrap();
        let first_snapshot_id = stored.snapshot_id.clone();
        let first_state_hash = stored.metadata.state_hash.clone();

        let second = manager.handle_checkpoint(
            &home,
            &workspace,
            &debounced_checkpoint(2, vec![PathBuf::from("phantom-watcher-path.txt")]),
        );

        assert_eq!(
            second.outcome,
            BackgroundCheckpointOutcome::Unchanged {
                snapshot_id: first_snapshot_id.clone(),
                state_hash: first_state_hash.clone(),
            }
        );
        assert_eq!(second.event.status, ProtectionStatus::Unchanged);
        let store = SnapshotStore::open(&home, "12345678").unwrap();
        assert_eq!(store.list_snapshots().unwrap().len(), 1);
    }
}

mod lease_epoch_monotonic {
    //! Invariant: `safety/lease_epoch_monotonic`; see `docs/data-loss-safety.md`.

    use super::*;

    fn complete_handoff(
        db: &mut MetadataDb,
        lease_id: &str,
        source_device: &str,
        target_device: &str,
        generation: &str,
    ) {
        let handoff = db
            .begin_handoff(lease_id, source_device, target_device, generation, 600)
            .unwrap();
        db.mark_handoff_target_verified(&handoff.handoff_id)
            .unwrap();
        db.mark_handoff_source_ready(&handoff.handoff_id).unwrap();
        db.commit_handoff(
            &handoff.handoff_id,
            generation,
            handoff.expires_at_unix_seconds.saturating_sub(1),
        )
        .unwrap();
    }

    #[test]
    fn committed_handoffs_advance_epoch_by_one_and_never_reuse_old_epoch() {
        let (_temp, mut db, _session_id, lease) = anchor_db();
        assert_eq!(lease.epoch, 2);

        let first = db
            .begin_handoff(&lease.lease_id, "device-a", "device-b", "gen-1", 600)
            .unwrap();
        assert_eq!(first.expected_epoch, 2);
        assert_eq!(db.get_lease(&lease.lease_id).unwrap().unwrap().epoch, 2);
        db.mark_handoff_target_verified(&first.handoff_id).unwrap();
        db.mark_handoff_source_ready(&first.handoff_id).unwrap();
        db.commit_handoff(
            &first.handoff_id,
            "gen-1",
            first.expires_at_unix_seconds.saturating_sub(1),
        )
        .unwrap();

        let after_first = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(after_first.epoch, 3);
        assert_eq!(after_first.holder_device_id.as_deref(), Some("device-b"));

        let stale_retry = db
            .commit_handoff(
                &first.handoff_id,
                "gen-1",
                first.expires_at_unix_seconds.saturating_sub(1),
            )
            .unwrap_err();
        assert!(stale_retry.to_string().contains("not source-ready"));
        assert_eq!(db.get_lease(&lease.lease_id).unwrap().unwrap().epoch, 3);

        complete_handoff(&mut db, &lease.lease_id, "device-b", "device-c", "gen-2");
        let after_second = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(after_second.epoch, 4);
        assert_eq!(after_second.holder_device_id.as_deref(), Some("device-c"));
    }
}

mod published_snapshots_immutable {
    //! Invariant: `safety/published_snapshots_immutable`; see `docs/data-loss-safety.md`.

    use super::*;

    #[test]
    fn duplicate_published_snapshot_id_cannot_replace_existing_metadata() {
        let (_temp, mut db, session_id, lease) = anchor_db();
        let mut original = publish_metadata("s1_000000000000000000000301", &session_id);
        original.project_name = "Original Safety Project".to_string();
        db.publish_snapshot_canonical(CanonicalPublishRequest {
            lease_id: &lease.lease_id,
            session_id: &session_id,
            expected_epoch: 2,
            holder_device_id: "device-a",
            expected_latest_snapshot_id: None,
            metadata: &original,
            pinned: false,
            label: Some("original"),
        })
        .unwrap();

        let mut replacement = original.clone();
        replacement.project_name = "Tampered Safety Project".to_string();
        let duplicate = db.publish_snapshot_canonical(CanonicalPublishRequest {
            lease_id: &lease.lease_id,
            session_id: &session_id,
            expected_epoch: 2,
            holder_device_id: "device-a",
            expected_latest_snapshot_id: Some(original.snapshot_id.as_str()),
            metadata: &replacement,
            pinned: true,
            label: Some("replacement"),
        });

        assert!(duplicate.is_err());
        let snapshots = db.list_stored_snapshots(Some("project123")).unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].snapshot_id, original.snapshot_id);
        assert_eq!(snapshots[0].metadata.project_name, original.project_name);
        assert!(!snapshots[0].pinned);
        assert_eq!(snapshots[0].label.as_deref(), Some("original"));
        let lease = db.get_lease(&lease.lease_id).unwrap().unwrap();
        assert_eq!(
            lease.latest_snapshot_id.as_deref(),
            Some(original.snapshot_id.as_str())
        );
    }
}
