//! Local synthetic snapshot creation, application, and verification.
//!
//! A snapshot records Git HEAD, the current index tree, a synthetic work-tree
//! commit, included untracked paths, excluded path reasons, and a state hash.
//! Applying a snapshot refuses dirty targets, fetches the synthetic refs, then
//! verifies HEAD, index tree, work tree, and state hash after materialization.

use crate::error::{DevRelayError, Result};
use crate::fs_safety::reparse_points_in_workspace;
use crate::path_doctor::{
    PathEntry, PathPortabilityIssue, PathPortabilityIssueCode, PathPortabilityPathSource,
    analyze_path_entries,
};
use crate::platform::{current_platform_key, platform_capabilities_for_key};
use crate::policy::classify_untracked_paths;
use crate::snapshot_schema::{SNAPSHOT_ID_PREFIX, SNAPSHOT_SCHEMA_VERSION, calculate_state_hash};
use crate::{
    CasStore, DEFAULT_SIDECAR_CHUNK_BYTES, GitRepo, GitStatus, Manifest, PathDecision,
    SnapshotMetadata, capture_large_sidecars,
};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotApplyFaultPoint {
    AfterTargetFetch,
    AfterBaseApply,
    AfterWorkApply,
    AfterIndexApply,
    DuringVerification,
}

impl SnapshotApplyFaultPoint {
    fn as_str(self) -> &'static str {
        match self {
            Self::AfterTargetFetch => "after-target-fetch",
            Self::AfterBaseApply => "after-base-apply",
            Self::AfterWorkApply => "after-work-apply",
            Self::AfterIndexApply => "after-index-apply",
            Self::DuringVerification => "during-verification",
        }
    }
}

pub fn create_snapshot(repo: &GitRepo, manifest: &Manifest) -> Result<SnapshotMetadata> {
    create_snapshot_inner(repo, manifest, None)
}

pub fn create_snapshot_with_sidecars(
    repo: &GitRepo,
    manifest: &Manifest,
    cas_store: &CasStore,
) -> Result<SnapshotMetadata> {
    create_snapshot_inner(repo, manifest, Some(cas_store))
}

fn create_snapshot_inner(
    repo: &GitRepo,
    manifest: &Manifest,
    cas_store: Option<&CasStore>,
) -> Result<SnapshotMetadata> {
    let status = repo.status()?;
    ensure_checkpoint_supported(repo, &status)?;
    let classified = classify_untracked_paths(repo.path(), manifest, status.untracked_paths())?;
    let sidecars = if let Some(cas_store) = cas_store {
        capture_large_sidecars(
            repo.path(),
            &classified,
            cas_store,
            DEFAULT_SIDECAR_CHUNK_BYTES,
        )?
    } else {
        Vec::new()
    };
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
        operation_capsule: None,
        included_untracked,
        excluded,
        sidecars,
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
    apply_snapshot_inner(target, source, snapshot, None)
}

pub fn apply_snapshot_with_fault_injection(
    target: &GitRepo,
    source: &GitRepo,
    snapshot: &SnapshotMetadata,
    fault: SnapshotApplyFaultPoint,
) -> Result<VerificationDetails> {
    apply_snapshot_inner(target, source, snapshot, Some(fault))
}

fn apply_snapshot_inner(
    target: &GitRepo,
    source: &GitRepo,
    snapshot: &SnapshotMetadata,
    fault: Option<SnapshotApplyFaultPoint>,
) -> Result<VerificationDetails> {
    plan_apply_snapshot(target, source, snapshot)?;
    let target_platform_key = current_platform_key();
    ensure_no_reparse_points_before_materialization(target)?;
    ensure_snapshot_paths_supported(source, snapshot, &target_platform_key)?;
    ensure_snapshot_materialization_supported(source, snapshot, &target_platform_key)?;
    fetch_snapshot_refs(target, source, snapshot)?;
    inject_apply_fault(fault, SnapshotApplyFaultPoint::AfterTargetFetch)?;

    if let Some(branch) = &snapshot.branch {
        target.run(&["checkout", "-B", branch, &snapshot.head_oid])?;
    } else {
        target.run(&["checkout", "--detach", &snapshot.head_oid])?;
    }
    target.run(&["reset", "--hard", &snapshot.head_oid])?;
    inject_apply_fault(fault, SnapshotApplyFaultPoint::AfterBaseApply)?;
    target.run(&["read-tree", "--reset", "-u", &snapshot.work_commit_oid])?;
    inject_apply_fault(fault, SnapshotApplyFaultPoint::AfterWorkApply)?;
    target.run(&["read-tree", "--reset", &snapshot.index_commit_oid])?;
    inject_apply_fault(fault, SnapshotApplyFaultPoint::AfterIndexApply)?;
    inject_apply_fault(fault, SnapshotApplyFaultPoint::DuringVerification)?;
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

fn inject_apply_fault(
    configured: Option<SnapshotApplyFaultPoint>,
    fault: SnapshotApplyFaultPoint,
) -> Result<()> {
    if configured == Some(fault) {
        return Err(DevRelayError::Config(format!(
            "injected apply fault at {}",
            fault.as_str()
        )));
    }
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

fn ensure_snapshot_paths_supported(
    repo: &GitRepo,
    snapshot: &SnapshotMetadata,
    target_platform_key: &str,
) -> Result<()> {
    let target_capabilities = platform_capabilities_for_key(target_platform_key);
    let mut entries = path_entries_in_tree(repo, &snapshot.index_tree_oid)?;
    entries.extend(path_entries_in_tree(repo, &snapshot.work_tree_oid)?);
    let mut issues = analyze_path_entries(&entries, &target_capabilities)
        .into_iter()
        .filter(|issue| snapshot_path_issue_blocks_target(issue, target_platform_key))
        .collect::<Vec<_>>();

    if issues.is_empty() {
        return Ok(());
    }

    issues.sort_by(|left, right| {
        left.path
            .cmp(&right.path)
            .then(issue_code_label(left.code).cmp(issue_code_label(right.code)))
    });
    issues.dedup_by(|left, right| left.code == right.code && left.path == right.path);
    let summary = issues
        .iter()
        .map(|issue| format!("{} ({})", issue.path, issue_code_label(issue.code)))
        .collect::<Vec<_>>()
        .join(", ");

    Err(DevRelayError::UnsupportedRepositoryState(format!(
        "target platform {target_platform_key} cannot materialize unsafe snapshot paths: {summary}"
    )))
}

fn snapshot_path_issue_blocks_target(
    issue: &PathPortabilityIssue,
    target_platform_key: &str,
) -> bool {
    let target_capabilities = platform_capabilities_for_key(target_platform_key);
    match issue.code {
        PathPortabilityIssueCode::CaseFoldCollision
        | PathPortabilityIssueCode::UnicodeNormalizationCollision => {
            !target_capabilities.case_sensitive_paths
        }
        PathPortabilityIssueCode::WindowsReservedName
        | PathPortabilityIssueCode::WindowsTrailingDotOrSpace
        | PathPortabilityIssueCode::WindowsInvalidCharacter
        | PathPortabilityIssueCode::PathLengthBudget => {
            target_platform_key.starts_with("windows-native-")
        }
        PathPortabilityIssueCode::SymlinkUnsupportedOnTarget => false,
    }
}

fn issue_code_label(code: PathPortabilityIssueCode) -> &'static str {
    match code {
        PathPortabilityIssueCode::CaseFoldCollision => "case-fold-collision",
        PathPortabilityIssueCode::UnicodeNormalizationCollision => {
            "unicode-normalization-collision"
        }
        PathPortabilityIssueCode::WindowsReservedName => "windows-reserved-name",
        PathPortabilityIssueCode::WindowsTrailingDotOrSpace => "windows-trailing-dot-or-space",
        PathPortabilityIssueCode::WindowsInvalidCharacter => "windows-invalid-character",
        PathPortabilityIssueCode::PathLengthBudget => "path-length-budget",
        PathPortabilityIssueCode::SymlinkUnsupportedOnTarget => "symlink-unsupported-on-target",
    }
}

fn ensure_no_reparse_points_before_materialization(target: &GitRepo) -> Result<()> {
    let mut points = reparse_points_in_workspace(target.path())?;
    if points.is_empty() {
        return Ok(());
    }
    points.sort();
    let paths = points
        .iter()
        .map(|path| path.to_string_lossy())
        .collect::<Vec<_>>()
        .join(", ");
    Err(DevRelayError::UnsupportedRepositoryState(format!(
        "target workspace contains Windows reparse points that DevRelay will not traverse: {paths}"
    )))
}

fn path_entries_in_tree(repo: &GitRepo, treeish: &str) -> Result<Vec<PathEntry>> {
    let raw = repo.run(&["ls-tree", "-r", "-z", treeish])?;
    let mut entries = Vec::new();
    for record in raw.split('\0').filter(|record| !record.is_empty()) {
        let Some((metadata, path)) = record.split_once('\t') else {
            return Err(DevRelayError::Config(format!(
                "unexpected git ls-tree record: {record:?}"
            )));
        };
        let mode = metadata.split_whitespace().next().unwrap_or_default();
        entries.push(PathEntry {
            path: path.replace('\\', "/"),
            source: PathPortabilityPathSource::Tracked,
            symlink: mode == "120000",
        });
    }
    Ok(entries)
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
    if let Some(state) = unsupported_operation_state(repo)? {
        return Err(DevRelayError::UnsupportedRepositoryState(state));
    }
    ensure_status_supports_checkpoint(status)?;
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
                "{name} state is not supported for checkpoint; finish, abort, or recover the operation before handoff"
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

    fn clone_repo(source_path: &Path, target_path: &Path) {
        GitRepo::new(source_path)
            .run_with_env(
                [
                    OsString::from("clone"),
                    source_path.as_os_str().to_os_string(),
                    target_path.as_os_str().to_os_string(),
                ],
                &[],
            )
            .unwrap();
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
    fn snapshot_with_sidecars_captures_large_untracked_file_in_cas() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = temp.path().join("repo");
        let repo = init_repo(&repo_path);
        commit_base(&repo, &repo_path);
        let cas = CasStore::open(temp.path().join("cas")).unwrap();
        let manifest = Manifest::parse(
            r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"
large_file_threshold_mib = 1
"#,
        )
        .unwrap();
        let large = vec![7_u8; 1024 * 1024 + 17];
        fs::write(repo_path.join("large.bin"), &large).unwrap();

        let snapshot = create_snapshot_with_sidecars(&repo, &manifest, &cas).unwrap();

        assert!(snapshot.included_untracked.is_empty());
        assert!(snapshot.excluded.iter().any(|item| {
            item.path == "large.bin"
                && item.reason == crate::classification_reason::LARGE_FILE_THRESHOLD
        }));
        assert_eq!(snapshot.sidecars.len(), 1);
        let sidecar = &snapshot.sidecars[0];
        assert_eq!(sidecar.logical_path, "large.bin");
        assert_eq!(
            sidecar.classification,
            crate::classification_reason::LARGE_FILE_THRESHOLD
        );
        assert_eq!(sidecar.size_bytes, large.len() as u64);
        assert_eq!(sidecar.chunk_size_bytes, DEFAULT_SIDECAR_CHUNK_BYTES as u64);
        assert_eq!(sidecar.root_hash, sidecar.cas_manifest_id);

        let manifest = cas.fetch_manifest(&sidecar.cas_manifest_id).unwrap();
        assert!(manifest.chunks.len() >= 2);
        let mut reconstructed = Vec::new();
        for chunk in manifest.chunks {
            reconstructed.extend(cas.download_chunk(&chunk.hash).unwrap());
        }
        assert_eq!(reconstructed, large);
        assert_eq!(snapshot.state_hash, calculate_state_hash(&snapshot));
        snapshot.validate().unwrap();
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
    fn checkpoint_rejects_rebase_and_sequencer_states() {
        for marker in ["rebase-merge", "rebase-apply", "sequencer"] {
            let temp = tempfile::tempdir().unwrap();
            let source_path = temp.path().join("source");
            let source = init_repo(&source_path);
            commit_base(&source, &source_path);
            fs::create_dir(source.git_dir().unwrap().join(marker)).unwrap();

            let err = create_snapshot(&source, &manifest()).unwrap_err();
            assert!(matches!(err, DevRelayError::UnsupportedRepositoryState(_)));
            assert!(err.to_string().contains(marker));
            assert!(err.to_string().contains("finish, abort, or recover"));
        }
    }

    #[test]
    fn checkpoint_rejects_merge_cherry_pick_and_revert_states_before_unmerged_status() {
        for (name, marker) in [
            ("merge", "MERGE_HEAD"),
            ("cherry-pick", "CHERRY_PICK_HEAD"),
            ("revert", "REVERT_HEAD"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let source_path = temp.path().join("source");
            let source = init_repo(&source_path);
            commit_base(&source, &source_path);
            fs::write(source.git_dir().unwrap().join(marker), "pending\n").unwrap();
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

            let err = ensure_checkpoint_supported(&source, &status).unwrap_err();

            assert!(matches!(err, DevRelayError::UnsupportedRepositoryState(_)));
            assert!(err.to_string().contains(name));
            assert!(err.to_string().contains("finish, abort, or recover"));
        }
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
    fn apply_fault_injection_preserves_source_and_snapshot_refs() {
        for (fault, label) in [
            (
                SnapshotApplyFaultPoint::AfterTargetFetch,
                "after-target-fetch",
            ),
            (SnapshotApplyFaultPoint::AfterBaseApply, "after-base-apply"),
            (SnapshotApplyFaultPoint::AfterWorkApply, "after-work-apply"),
            (
                SnapshotApplyFaultPoint::AfterIndexApply,
                "after-index-apply",
            ),
            (
                SnapshotApplyFaultPoint::DuringVerification,
                "during-verification",
            ),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let source_path = temp.path().join("source");
            let target_path = temp.path().join("target");
            let source = init_repo(&source_path);
            commit_base(&source, &source_path);
            fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
            fs::write(source_path.join("notes.md"), "carry me\n").unwrap();
            let snapshot = create_snapshot(&source, &manifest()).unwrap();
            clone_repo(&source_path, &target_path);
            let target = GitRepo::new(&target_path);
            let before_head = target.run(&["rev-parse", "HEAD"]).unwrap();
            let before_status = target.status().unwrap();

            let err = apply_snapshot_with_fault_injection(&target, &source, &snapshot, fault)
                .unwrap_err();

            assert!(
                err.to_string()
                    .contains(&format!("injected apply fault at {label}")),
                "{fault:?} returned {err}"
            );
            assert_eq!(
                fs::read_to_string(source_path.join("tracked.txt")).unwrap(),
                "changed\n"
            );
            assert_eq!(
                fs::read_to_string(source_path.join("notes.md")).unwrap(),
                "carry me\n"
            );
            for git_ref in [snapshot.index_ref(), snapshot.work_ref()] {
                assert!(
                    source.run(&["rev-parse", "--verify", &git_ref]).is_ok(),
                    "source ref {git_ref} should remain available after {fault:?}"
                );
                assert!(
                    target.run(&["rev-parse", "--verify", &git_ref]).is_ok(),
                    "target ref {git_ref} should be fetched before {fault:?}"
                );
            }

            if fault == SnapshotApplyFaultPoint::AfterTargetFetch {
                assert_eq!(target.run(&["rev-parse", "HEAD"]).unwrap(), before_head);
                assert_eq!(target.status().unwrap(), before_status);
                assert!(!target_path.join("notes.md").exists());
            }
            if matches!(
                fault,
                SnapshotApplyFaultPoint::AfterIndexApply
                    | SnapshotApplyFaultPoint::DuringVerification
            ) {
                verify_snapshot(&target, &snapshot).unwrap();
            }
        }
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

    #[test]
    fn blocks_windows_unsafe_snapshot_paths_before_materialization() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        commit_base(&source, &source_path);
        fs::write(source_path.join("CON.txt"), "reserved\n").unwrap();
        source.run(&["add", "CON.txt"]).unwrap();
        fs::write(source_path.join("scratch?.txt"), "accepted untracked\n").unwrap();

        let snapshot = create_snapshot(&source, &manifest()).unwrap();
        let err = ensure_snapshot_paths_supported(&source, &snapshot, "windows-native-x86_64")
            .unwrap_err();

        assert!(matches!(err, DevRelayError::UnsupportedRepositoryState(_)));
        assert!(err.to_string().contains("CON.txt"));
        assert!(err.to_string().contains("windows-reserved-name"));
        assert!(err.to_string().contains("scratch?.txt"));
        assert!(err.to_string().contains("windows-invalid-character"));
    }

    #[test]
    fn allows_windows_specific_path_names_on_non_windows_targets() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        commit_base(&source, &source_path);
        fs::write(source_path.join("CON.txt"), "reserved on windows only\n").unwrap();
        source.run(&["add", "CON.txt"]).unwrap();

        let snapshot = create_snapshot(&source, &manifest()).unwrap();

        ensure_snapshot_paths_supported(&source, &snapshot, "linux-gnu-x86_64").unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn blocks_windows_reparse_points_before_materialization() {
        let temp = tempfile::tempdir().unwrap();
        let repo_path = temp.path().join("repo");
        let junction_target = temp.path().join("junction-target");
        let junction = repo_path.join("junction");
        let repo = init_repo(&repo_path);
        commit_base(&repo, &repo_path);
        fs::create_dir(&junction_target).unwrap();

        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&junction_target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "mklink /J failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let err = ensure_no_reparse_points_before_materialization(&repo).unwrap_err();

        assert!(matches!(err, DevRelayError::UnsupportedRepositoryState(_)));
        assert!(err.to_string().contains("junction"));
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
