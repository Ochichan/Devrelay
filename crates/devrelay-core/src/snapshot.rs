//! Local synthetic snapshot creation, application, and verification.
//!
//! A snapshot records Git HEAD, the current index tree, a synthetic work-tree
//! commit, included untracked paths, excluded path reasons, and a state hash.
//! Applying a snapshot refuses dirty targets, fetches the synthetic refs, then
//! verifies HEAD, index tree, work tree, and state hash after materialization.

use crate::error::{DevRelayError, Result};
use crate::platform::{current_platform_key, platform_capabilities_for_key};
use crate::policy::classify_untracked_paths;
use crate::snapshot_schema::{SNAPSHOT_ID_PREFIX, SNAPSHOT_SCHEMA_VERSION, calculate_state_hash};
use crate::{GitRepo, GitStatus, Manifest, PathDecision, SnapshotMetadata};
use serde::{Deserialize, Serialize};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApplyPlan {
    pub snapshot_id: String,
    pub branch: Option<String>,
    pub detached: bool,
    pub head_oid: String,
    pub index_ref: String,
    pub work_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationDetails {
    pub head_oid: String,
    pub index_tree_oid: String,
    pub work_tree_oid: String,
    pub state_hash: String,
    pub included_untracked: Vec<String>,
    pub excluded_paths: Vec<String>,
}

pub fn create_snapshot(repo: &GitRepo, manifest: &Manifest) -> Result<SnapshotMetadata> {
    let status = repo.status()?;
    ensure_checkpoint_supported(repo, &status)?;
    let classified = classify_untracked_paths(repo.path(), manifest, status.untracked_paths())?;
    let mut included_untracked = classified
        .iter()
        .filter(|item| item.decision == PathDecision::Include)
        .map(|item| item.path.clone())
        .collect::<Vec<_>>();
    included_untracked.sort();
    let mut excluded = classified
        .into_iter()
        .filter(|item| item.decision == PathDecision::Exclude)
        .collect::<Vec<_>>();
    excluded.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(left.reason.cmp(&right.reason))
    });

    let head_oid = status.head_oid.clone();
    let index_tree_oid = repo.current_index_tree()?;
    let index_commit_oid = commit_tree(
        repo,
        &index_tree_oid,
        Some(&head_oid),
        "DevRelay synthetic index snapshot",
    )?;
    let work_tree_oid = write_work_tree(repo, &included_untracked)?;
    let work_commit_oid = commit_tree(
        repo,
        &work_tree_oid,
        Some(&index_commit_oid),
        "DevRelay synthetic work snapshot",
    )?;

    let snapshot_id = snapshot_id(&status, &index_tree_oid, &work_tree_oid);
    let metadata = SnapshotMetadata {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        snapshot_id,
        project_id: manifest.project_id.clone(),
        project_name: manifest.name.clone(),
        session_id: None,
        parent_snapshot_id: None,
        source_device_id: None,
        branch: status.branch.clone(),
        head_oid,
        index_tree_oid,
        index_commit_oid,
        work_tree_oid,
        work_commit_oid,
        source_status: status.counts,
        included_untracked,
        excluded,
        state_hash: String::new(),
        created_at_unix_seconds: unix_seconds(),
    };
    let metadata = SnapshotMetadata {
        state_hash: calculate_state_hash(&metadata),
        ..metadata
    };
    metadata.validate()?;

    repo.run(&[
        "update-ref",
        &metadata.index_ref(),
        &metadata.index_commit_oid,
    ])?;
    repo.run(&[
        "update-ref",
        &metadata.work_ref(),
        &metadata.work_commit_oid,
    ])?;

    Ok(metadata)
}

pub fn write_snapshot_file(path: impl AsRef<Path>, metadata: &SnapshotMetadata) -> Result<()> {
    metadata.validate()?;
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(metadata)?)?;
    Ok(())
}

pub fn read_snapshot_file(path: impl AsRef<Path>) -> Result<SnapshotMetadata> {
    let raw = fs::read_to_string(path)?;
    let metadata: SnapshotMetadata = serde_json::from_str(&raw)?;
    metadata.validate()?;
    Ok(metadata)
}

pub fn apply_snapshot(
    target: &GitRepo,
    source: &GitRepo,
    snapshot: &SnapshotMetadata,
) -> Result<VerificationDetails> {
    plan_apply_snapshot(target, source, snapshot)?;
    ensure_snapshot_materialization_supported(source, snapshot, &current_platform_key())?;
    fetch_snapshot_refs(target, source, snapshot)?;

    if let Some(branch) = &snapshot.branch {
        target.run(&["checkout", "-B", branch, &snapshot.head_oid])?;
    } else {
        target.run(&["checkout", "--detach", &snapshot.head_oid])?;
    }
    target.run(&["reset", "--hard", &snapshot.head_oid])?;
    target.run(&["read-tree", "--reset", "-u", &snapshot.work_commit_oid])?;
    target.run(&["read-tree", "--reset", &snapshot.index_commit_oid])?;
    verify_snapshot(target, snapshot)
}

pub fn plan_apply_snapshot(
    target: &GitRepo,
    source: &GitRepo,
    snapshot: &SnapshotMetadata,
) -> Result<ApplyPlan> {
    let target_status = target.status()?;
    if !target_status.is_clean() {
        return Err(DevRelayError::TargetDirty(target_status.short_summary()));
    }

    ensure_source_snapshot_refs(source, snapshot)?;

    Ok(ApplyPlan {
        snapshot_id: snapshot.snapshot_id.clone(),
        branch: snapshot.branch.clone(),
        detached: snapshot.branch.is_none(),
        head_oid: snapshot.head_oid.clone(),
        index_ref: snapshot.index_ref(),
        work_ref: snapshot.work_ref(),
    })
}

pub fn verify_snapshot(repo: &GitRepo, snapshot: &SnapshotMetadata) -> Result<VerificationDetails> {
    let head = repo.run(&["rev-parse", "HEAD"])?;
    if head != snapshot.head_oid {
        return Err(DevRelayError::Verification(format!(
            "HEAD mismatch: expected {}, got {}",
            snapshot.head_oid, head
        )));
    }

    let index_tree = repo.current_index_tree()?;
    if index_tree != snapshot.index_tree_oid {
        return Err(DevRelayError::Verification(format!(
            "index tree mismatch: expected {}, got {}",
            snapshot.index_tree_oid, index_tree
        )));
    }

    let work_tree = write_work_tree(repo, &snapshot.included_untracked)?;
    if work_tree != snapshot.work_tree_oid {
        return Err(DevRelayError::Verification(format!(
            "work tree mismatch: expected {}, got {}",
            snapshot.work_tree_oid, work_tree
        )));
    }

    let calculated = calculate_state_hash(snapshot);
    if calculated != snapshot.state_hash {
        return Err(DevRelayError::Verification(format!(
            "state hash mismatch: expected {}, got {}",
            snapshot.state_hash, calculated
        )));
    }

    verify_included_untracked_paths(repo, snapshot)?;
    verify_excluded_paths_absent(repo, snapshot)?;

    Ok(VerificationDetails {
        head_oid: head,
        index_tree_oid: index_tree,
        work_tree_oid: work_tree,
        state_hash: calculated,
        included_untracked: snapshot.included_untracked.clone(),
        excluded_paths: snapshot
            .excluded
            .iter()
            .map(|item| item.path.clone())
            .collect(),
    })
}

fn fetch_snapshot_refs(
    target: &GitRepo,
    source: &GitRepo,
    snapshot: &SnapshotMetadata,
) -> Result<()> {
    let source_path = source.path().as_os_str().to_os_string();
    target
        .run_with_env(
            [
                OsString::from("fetch"),
                source_path,
                OsString::from(format!("{}:{}", snapshot.index_ref(), snapshot.index_ref())),
                OsString::from(format!("{}:{}", snapshot.work_ref(), snapshot.work_ref())),
            ],
            &[],
        )
        .map_err(|err| DevRelayError::MissingSourceObject(err.to_string()))?;
    Ok(())
}

fn ensure_source_snapshot_refs(source: &GitRepo, snapshot: &SnapshotMetadata) -> Result<()> {
    for git_ref in [snapshot.index_ref(), snapshot.work_ref()] {
        source
            .run(&["rev-parse", "--verify", &git_ref])
            .map_err(|err| {
                DevRelayError::MissingSourceObject(format!("missing source ref {git_ref}: {err}"))
            })?;
    }
    Ok(())
}

fn ensure_snapshot_materialization_supported(
    repo: &GitRepo,
    snapshot: &SnapshotMetadata,
    target_platform_key: &str,
) -> Result<()> {
    if platform_capabilities_for_key(target_platform_key).symlinks {
        return Ok(());
    }

    let mut symlink_paths = symlink_paths_in_tree(repo, &snapshot.index_tree_oid)?;
    symlink_paths.extend(symlink_paths_in_tree(repo, &snapshot.work_tree_oid)?);
    symlink_paths.sort();
    symlink_paths.dedup();
    if symlink_paths.is_empty() {
        return Ok(());
    }

    Err(DevRelayError::UnsupportedRepositoryState(format!(
        "target platform {target_platform_key} does not support symlink materialization for: {}",
        symlink_paths.join(", ")
    )))
}

fn symlink_paths_in_tree(repo: &GitRepo, treeish: &str) -> Result<Vec<String>> {
    let raw = repo.run(&["ls-tree", "-r", "-z", treeish])?;
    let mut paths = Vec::new();
    for record in raw.split('\0').filter(|record| !record.is_empty()) {
        let Some((metadata, path)) = record.split_once('\t') else {
            return Err(DevRelayError::Config(format!(
                "unexpected git ls-tree record: {record:?}"
            )));
        };
        let mode = metadata.split_whitespace().next().unwrap_or_default();
        if mode == "120000" {
            paths.push(path.replace('\\', "/"));
        }
    }
    Ok(paths)
}

fn verify_included_untracked_paths(repo: &GitRepo, snapshot: &SnapshotMetadata) -> Result<()> {
    for path in &snapshot.included_untracked {
        if !repo.path().join(PathBuf::from(path)).exists() {
            return Err(DevRelayError::Verification(format!(
                "included untracked path missing after apply: {path}"
            )));
        }
    }
    Ok(())
}

fn verify_excluded_paths_absent(repo: &GitRepo, snapshot: &SnapshotMetadata) -> Result<()> {
    for item in &snapshot.excluded {
        if repo.path().join(PathBuf::from(&item.path)).exists() {
            return Err(DevRelayError::Verification(format!(
                "excluded path materialized after apply: {}",
                item.path
            )));
        }
    }
    Ok(())
}

fn write_work_tree(repo: &GitRepo, included_untracked: &[String]) -> Result<String> {
    let temp_parent = repo.git_dir()?.join("devrelay-tmp");
    fs::create_dir_all(&temp_parent)?;
    let temp_dir = tempfile::Builder::new()
        .prefix("work-index-")
        .tempdir_in(&temp_parent)?;
    let temp_index = temp_dir.path().join("index");
    let tree = write_work_tree_with_temp_index(repo, included_untracked, &temp_index);
    drop(temp_dir);
    let _ = fs::remove_dir(&temp_parent);
    tree
}

fn write_work_tree_with_temp_index(
    repo: &GitRepo,
    included_untracked: &[String],
    temp_index: &Path,
) -> Result<String> {
    copy_current_index(repo, temp_index)?;

    git_with_index(repo, temp_index, ["add", "-u", "--"])?;
    if !included_untracked.is_empty() {
        let mut args = vec![OsString::from("add"), OsString::from("--")];
        args.extend(included_untracked.iter().map(OsString::from));
        repo.run_with_env(args, &[("GIT_INDEX_FILE", temp_index.as_os_str())])?;
    }
    git_with_index(repo, temp_index, ["write-tree"])
}

fn copy_current_index(repo: &GitRepo, target: &Path) -> Result<()> {
    let source = repo.git_dir()?.join("index");
    if source.exists() {
        fs::copy(source, target)?;
    } else {
        fs::File::create(target)?;
    }
    Ok(())
}

fn git_with_index<const N: usize>(
    repo: &GitRepo,
    index: &Path,
    args: [&'static str; N],
) -> Result<String> {
    repo.run_with_env(
        args.into_iter().map(OsString::from),
        &[("GIT_INDEX_FILE", index.as_os_str())],
    )
}

fn commit_tree(
    repo: &GitRepo,
    tree_oid: &str,
    parent_oid: Option<&str>,
    message: &str,
) -> Result<String> {
    let mut args = vec![OsString::from("commit-tree"), OsString::from(tree_oid)];
    if let Some(parent_oid) = parent_oid {
        args.push(OsString::from("-p"));
        args.push(OsString::from(parent_oid));
    }
    args.push(OsString::from("-m"));
    args.push(OsString::from(message));

    repo.run_with_env(
        args,
        &[
            ("GIT_AUTHOR_NAME", OsStr::new("DevRelay")),
            ("GIT_AUTHOR_EMAIL", OsStr::new("devrelay@local")),
            ("GIT_COMMITTER_NAME", OsStr::new("DevRelay")),
            ("GIT_COMMITTER_EMAIL", OsStr::new("devrelay@local")),
        ],
    )
}

fn snapshot_id(status: &GitStatus, index_tree_oid: &str, work_tree_oid: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(status.head_oid.as_bytes());
    if let Some(branch) = &status.branch {
        hasher.update(branch.as_bytes());
    }
    hasher.update(index_tree_oid.as_bytes());
    hasher.update(work_tree_oid.as_bytes());
    hasher.update(&unix_nanos().to_le_bytes());
    let digest = hasher.finalize();
    format!("{}{}", SNAPSHOT_ID_PREFIX, &digest.to_hex()[..24])
}

fn ensure_checkpoint_supported(repo: &GitRepo, status: &GitStatus) -> Result<()> {
    ensure_status_supports_checkpoint(status)?;
    if let Some(state) = unsupported_operation_state(repo)? {
        return Err(DevRelayError::UnsupportedRepositoryState(state));
    }
    Ok(())
}

fn ensure_status_supports_checkpoint(status: &GitStatus) -> Result<()> {
    if status.is_initial() {
        return Err(DevRelayError::UnsupportedRepositoryState(
            "repository has no HEAD commit".to_string(),
        ));
    }
    if status.counts.unmerged > 0 {
        return Err(DevRelayError::UnsupportedRepositoryState(
            "unmerged index entries are not supported in M0".to_string(),
        ));
    }
    Ok(())
}

fn unsupported_operation_state(repo: &GitRepo) -> Result<Option<String>> {
    let git_dir = repo.git_dir()?;
    for (name, relative_path) in [
        ("rebase-merge", "rebase-merge"),
        ("rebase-apply", "rebase-apply"),
        ("sequencer", "sequencer"),
        ("merge", "MERGE_HEAD"),
        ("cherry-pick", "CHERRY_PICK_HEAD"),
        ("revert", "REVERT_HEAD"),
    ] {
        if git_dir.join(relative_path).exists() {
            return Ok(Some(format!(
                "{name} state is not supported for checkpoint in M0"
            )));
        }
    }
    Ok(None)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StatusCounts, manifest::Manifest};
    use std::fs;

    fn manifest() -> Manifest {
        Manifest::parse(
            r#"
schema = 1
project_id = "12345678"
name = "roundtrip"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#,
        )
        .unwrap()
    }

    fn init_repo(path: &Path) -> GitRepo {
        fs::create_dir(path).unwrap();
        let repo = GitRepo::new(path);
        repo.run(&["init", "-b", "main"]).unwrap();
        repo.run(&["config", "user.name", "DevRelay Test"]).unwrap();
        repo.run(&["config", "user.email", "devrelay-test@example.local"])
            .unwrap();
        repo
    }

    fn commit_base(repo: &GitRepo, path: &Path) {
        fs::write(path.join("tracked.txt"), "base\n").unwrap();
        repo.run(&["add", "."]).unwrap();
        repo.run(&["commit", "-m", "base"]).unwrap();
    }

    #[test]
    fn creates_and_applies_snapshot_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let target_path = temp.path().join("target");
        let source = init_repo(&source_path);
        fs::write(source_path.join("tracked.txt"), "base\n").unwrap();
        fs::write(source_path.join("staged.txt"), "old\n").unwrap();
        source.run(&["add", "."]).unwrap();
        source.run(&["commit", "-m", "base"]).unwrap();

        fs::write(source_path.join("staged.txt"), "new staged\n").unwrap();
        source.run(&["add", "staged.txt"]).unwrap();
        fs::write(source_path.join("tracked.txt"), "base\nunstaged\n").unwrap();
        fs::write(source_path.join("notes.md"), "carry me\n").unwrap();
        fs::write(source_path.join(".env"), "DATABASE_URL=secret\n").unwrap();

        let snapshot = create_snapshot(&source, &manifest()).unwrap();
        assert_eq!(snapshot.included_untracked, vec!["notes.md"]);
        assert!(snapshot.excluded.iter().any(|item| item.path == ".env"));
        assert!(!source.git_dir().unwrap().join("devrelay-tmp").exists());

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
        let target = GitRepo::new(&target_path);
        let verification = apply_snapshot(&target, &source, &snapshot).unwrap();
        assert_eq!(verification.included_untracked, vec!["notes.md"]);
        assert!(verification.excluded_paths.contains(&".env".to_string()));
        verify_snapshot(&target, &snapshot).unwrap();

        let target_status = target.status().unwrap();
        assert_eq!(target_status.counts.staged, 1);
        assert_eq!(target_status.counts.unstaged, 1);
        assert_eq!(target_status.counts.untracked, 1);
        assert!(!target_path.join(".env").exists());
    }

    #[test]
    fn refuses_dirty_target() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let target_path = temp.path().join("target");
        let source = init_repo(&source_path);
        commit_base(&source, &source_path);
        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let snapshot = create_snapshot(&source, &manifest()).unwrap();

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
        let target = GitRepo::new(&target_path);
        fs::write(target_path.join("local.txt"), "do not overwrite\n").unwrap();
        let err = apply_snapshot(&target, &source, &snapshot).unwrap_err();
        assert!(matches!(err, DevRelayError::TargetDirty(_)));
        assert_eq!(err.code(), "DR-APPLY-DIRTY-TARGET");
    }

    #[test]
    fn checkpoint_preserves_source_worktree_and_index() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        fs::write(source_path.join("tracked.txt"), "base\n").unwrap();
        fs::write(source_path.join("staged.txt"), "old\n").unwrap();
        source.run(&["add", "."]).unwrap();
        source.run(&["commit", "-m", "base"]).unwrap();

        fs::write(source_path.join("staged.txt"), "new staged\n").unwrap();
        source.run(&["add", "staged.txt"]).unwrap();
        fs::write(source_path.join("tracked.txt"), "base\nunstaged\n").unwrap();
        fs::write(source_path.join("notes.md"), "carry me\n").unwrap();

        let before_status = source
            .run(&[
                "status",
                "--porcelain=v2",
                "-z",
                "--branch",
                "--untracked-files=all",
            ])
            .unwrap();
        let before_index_tree = source.current_index_tree().unwrap();

        create_snapshot(&source, &manifest()).unwrap();

        let after_status = source
            .run(&[
                "status",
                "--porcelain=v2",
                "-z",
                "--branch",
                "--untracked-files=all",
            ])
            .unwrap();
        let after_index_tree = source.current_index_tree().unwrap();

        assert_eq!(before_status, after_status);
        assert_eq!(before_index_tree, after_index_tree);
    }

    #[test]
    fn checkpoint_rejects_unborn_repository() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);

        let err = create_snapshot(&source, &manifest()).unwrap_err();
        assert!(matches!(err, DevRelayError::UnsupportedRepositoryState(_)));
        assert!(err.to_string().contains("no HEAD commit"));
    }

    #[test]
    fn checkpoint_rejects_unmerged_status() {
        let status = GitStatus {
            head_oid: "abc".to_string(),
            branch: Some("main".to_string()),
            upstream: None,
            entries: Vec::new(),
            counts: StatusCounts {
                unmerged: 1,
                ..StatusCounts::default()
            },
        };

        let err = ensure_status_supports_checkpoint(&status).unwrap_err();
        assert!(matches!(err, DevRelayError::UnsupportedRepositoryState(_)));
        assert!(err.to_string().contains("unmerged"));
    }

    #[test]
    fn checkpoint_rejects_rebase_state() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        commit_base(&source, &source_path);
        fs::create_dir(source.git_dir().unwrap().join("rebase-merge")).unwrap();

        let err = create_snapshot(&source, &manifest()).unwrap_err();
        assert!(matches!(err, DevRelayError::UnsupportedRepositoryState(_)));
        assert!(err.to_string().contains("rebase-merge"));
    }

    #[test]
    fn apply_dry_run_returns_plan_without_mutating_target() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let target_path = temp.path().join("target");
        let source = init_repo(&source_path);
        commit_base(&source, &source_path);
        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let snapshot = create_snapshot(&source, &manifest()).unwrap();

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
        let target = GitRepo::new(&target_path);
        let before_head = target.run(&["rev-parse", "HEAD"]).unwrap();
        let before_status = target.status().unwrap();

        let plan = plan_apply_snapshot(&target, &source, &snapshot).unwrap();

        assert_eq!(plan.snapshot_id, snapshot.snapshot_id);
        assert_eq!(target.run(&["rev-parse", "HEAD"]).unwrap(), before_head);
        assert_eq!(target.status().unwrap(), before_status);
    }

    #[test]
    fn apply_reports_missing_source_object_with_stable_error() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let target_path = temp.path().join("target");
        let source = init_repo(&source_path);
        commit_base(&source, &source_path);
        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let snapshot = create_snapshot(&source, &manifest()).unwrap();

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
        let target = GitRepo::new(&target_path);
        let index_ref = snapshot.index_ref();
        source.run(&["update-ref", "-d", &index_ref]).unwrap();

        let err = plan_apply_snapshot(&target, &source, &snapshot).unwrap_err();
        assert!(matches!(err, DevRelayError::MissingSourceObject(_)));
        assert_eq!(err.code(), "DR-APPLY-MISSING-SOURCE-OBJECT");
    }

    #[test]
    fn verification_mismatch_uses_stable_error() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let target_path = temp.path().join("target");
        let source = init_repo(&source_path);
        commit_base(&source, &source_path);
        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let snapshot = create_snapshot(&source, &manifest()).unwrap();

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
        let target = GitRepo::new(&target_path);
        apply_snapshot(&target, &source, &snapshot).unwrap();

        let mut bad_snapshot = snapshot;
        bad_snapshot.state_hash = "bad".to_string();
        let err = verify_snapshot(&target, &bad_snapshot).unwrap_err();

        assert!(matches!(err, DevRelayError::Verification(_)));
        assert_eq!(err.code(), "DR-APPLY-VERIFICATION-MISMATCH");
    }

    #[cfg(unix)]
    #[test]
    fn blocks_symlink_materialization_for_targets_without_symlink_support() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        commit_base(&source, &source_path);
        std::os::unix::fs::symlink("tracked.txt", source_path.join("tracked-link")).unwrap();

        let snapshot = create_snapshot(&source, &manifest()).unwrap();
        let err =
            ensure_snapshot_materialization_supported(&source, &snapshot, "windows-native-x86_64")
                .unwrap_err();

        assert!(matches!(err, DevRelayError::UnsupportedRepositoryState(_)));
        assert!(err.to_string().contains("tracked-link"));
    }
}
