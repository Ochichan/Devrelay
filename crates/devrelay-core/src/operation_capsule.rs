//! In-progress Git operation capture primitives.
//!
//! These structures describe merge-like conflict state without applying it to
//! normal snapshots yet. M12 can use them as the durable operation capsule once
//! conflict round-trip support is exhaustive.

use crate::{DevRelayError, GitRepo, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub const REBASE_OPERATION_RECONSTRUCTION_ENABLED: bool = false;
pub const REBASE_OPERATION_MIN_TARGET_GIT_VERSION: Option<&str> = None;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationCapsule {
    pub operation: GitOperationMetadata,
    pub unmerged_entries: Vec<UnmergedIndexEntry>,
    #[serde(default)]
    pub worktree_files: Vec<ConflictWorktreeFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitOperationMetadata {
    pub kind: GitOperationKind,
    pub current_head_oid: String,
    pub operation_oids: Vec<String>,
    pub original_head_oid: Option<String>,
    #[serde(default)]
    pub progress: Option<GitOperationProgress>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum GitOperationKind {
    Merge,
    CherryPick,
    Revert,
    RebaseMerge,
    RebaseApply,
    Sequencer,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitOperationProgress {
    pub interactive: bool,
    pub original_head_oid: Option<String>,
    pub onto_oid: Option<String>,
    pub head_name: Option<String>,
    pub todo: Vec<String>,
    pub done: Vec<String>,
    pub current_step: Option<GitOperationStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitOperationStep {
    pub current: Option<u64>,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnmergedIndexEntry {
    pub path: String,
    pub stages: Vec<IndexStageEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexStageEntry {
    pub stage: u8,
    pub mode: String,
    pub oid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConflictWorktreeFile {
    pub path: String,
    pub contents: Vec<u8>,
}

pub fn capture_operation_capsule(repo: &GitRepo) -> Result<Option<OperationCapsule>> {
    let Some(operation) = capture_operation_metadata(repo)? else {
        return Ok(None);
    };
    let unmerged_entries = capture_unmerged_index_entries(repo)?;
    let worktree_files = capture_conflict_worktree_files(repo, &unmerged_entries)?;
    Ok(Some(OperationCapsule {
        operation,
        unmerged_entries,
        worktree_files,
    }))
}

pub fn apply_unmerged_index_entries(repo: &GitRepo, capsule: &OperationCapsule) -> Result<()> {
    if capsule.unmerged_entries.is_empty() {
        return Ok(());
    }

    for entry in &capsule.unmerged_entries {
        repo.run(&["update-index", "--force-remove", "--", &entry.path])?;
    }

    let mut index_info = String::new();
    for entry in &capsule.unmerged_entries {
        for stage in &entry.stages {
            index_info.push_str(&format!(
                "{} {} {}\t{}\n",
                stage.mode, stage.oid, stage.stage, entry.path
            ));
        }
    }
    repo.run_with_stdin(&["update-index", "--index-info"], index_info.as_bytes())?;
    Ok(())
}

pub fn restore_conflict_worktree_files(repo: &GitRepo, capsule: &OperationCapsule) -> Result<()> {
    for file in &capsule.worktree_files {
        let path = repo.path().join(PathBuf::from(&file.path));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, &file.contents)?;
    }
    Ok(())
}

fn capture_operation_metadata(repo: &GitRepo) -> Result<Option<GitOperationMetadata>> {
    let git_dir = repo.git_dir()?;
    let Some((kind, marker_path)) = [
        (GitOperationKind::Merge, "MERGE_HEAD"),
        (GitOperationKind::CherryPick, "CHERRY_PICK_HEAD"),
        (GitOperationKind::Revert, "REVERT_HEAD"),
        (GitOperationKind::RebaseMerge, "rebase-merge"),
        (GitOperationKind::RebaseApply, "rebase-apply"),
        (GitOperationKind::Sequencer, "sequencer"),
    ]
    .into_iter()
    .find(|(_, marker_path)| git_dir.join(marker_path).exists()) else {
        return Ok(None);
    };
    let marker = git_dir.join(marker_path);
    let progress = capture_operation_progress(kind, &marker)?;
    let original_head_oid = progress
        .as_ref()
        .and_then(|progress| progress.original_head_oid.clone())
        .or(optional_oid_file(&git_dir.join("ORIG_HEAD"))?);

    Ok(Some(GitOperationMetadata {
        kind,
        current_head_oid: repo.run(&["rev-parse", "HEAD"])?,
        operation_oids: marker_oids(&marker)?,
        original_head_oid,
        progress,
    }))
}

fn capture_operation_progress(
    kind: GitOperationKind,
    marker: &std::path::Path,
) -> Result<Option<GitOperationProgress>> {
    match kind {
        GitOperationKind::RebaseMerge | GitOperationKind::RebaseApply => {
            capture_rebase_progress(marker)
        }
        GitOperationKind::Sequencer => capture_sequencer_progress(marker),
        GitOperationKind::Merge | GitOperationKind::CherryPick | GitOperationKind::Revert => {
            Ok(None)
        }
    }
}

fn capture_rebase_progress(path: &std::path::Path) -> Result<Option<GitOperationProgress>> {
    let todo = optional_lines_file(&path.join("git-rebase-todo"))?;
    let done = optional_lines_file(&path.join("done"))?;
    let mut current = optional_u64_file(&path.join("msgnum"))?;
    let mut total = optional_u64_file(&path.join("end"))?;
    if current.is_none() || total.is_none() {
        let inferred = infer_step(&done, &todo);
        current = current.or(inferred.current);
        total = total.or(inferred.total);
    }

    Ok(Some(GitOperationProgress {
        interactive: path.join("interactive").exists(),
        original_head_oid: optional_oid_file(&path.join("orig-head"))?,
        onto_oid: optional_oid_file(&path.join("onto"))?,
        head_name: optional_trimmed_file(&path.join("head-name"))?,
        todo,
        done,
        current_step: (current.is_some() || total.is_some())
            .then_some(GitOperationStep { current, total }),
    }))
}

fn capture_sequencer_progress(path: &std::path::Path) -> Result<Option<GitOperationProgress>> {
    let todo = optional_lines_file(&path.join("todo"))?;
    let done = optional_lines_file(&path.join("done"))?;
    let inferred = infer_step(&done, &todo);

    Ok(Some(GitOperationProgress {
        interactive: false,
        original_head_oid: optional_oid_file(&path.join("head"))?,
        onto_oid: None,
        head_name: None,
        todo,
        done,
        current_step: (inferred.current.is_some() || inferred.total.is_some()).then_some(inferred),
    }))
}

fn marker_oids(path: &std::path::Path) -> Result<Vec<String>> {
    if path.is_dir() {
        return Ok(Vec::new());
    }
    Ok(fs::read_to_string(path)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect())
}

fn optional_lines_file(path: &std::path::Path) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    Ok(fs::read_to_string(path)?
        .lines()
        .map(str::to_string)
        .collect())
}

fn optional_oid_file(path: &std::path::Path) -> Result<Option<String>> {
    optional_trimmed_file(path)
}

fn optional_trimmed_file(path: &std::path::Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(fs::read_to_string(path)?
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string))
}

fn optional_u64_file(path: &std::path::Path) -> Result<Option<u64>> {
    let Some(value) = optional_trimmed_file(path)? else {
        return Ok(None);
    };
    Ok(Some(value.parse::<u64>().map_err(|_| {
        DevRelayError::Config(format!("unexpected numeric Git operation value: {value:?}"))
    })?))
}

fn infer_step(done: &[String], todo: &[String]) -> GitOperationStep {
    let done_count = todo_command_count(done);
    let todo_count = todo_command_count(todo);
    let total = done_count + todo_count;
    GitOperationStep {
        current: (total > 0).then_some((done_count + 1).min(total)),
        total: (total > 0).then_some(total),
    }
}

fn todo_command_count(lines: &[String]) -> u64 {
    lines
        .iter()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with('#')
        })
        .count() as u64
}

fn capture_unmerged_index_entries(repo: &GitRepo) -> Result<Vec<UnmergedIndexEntry>> {
    let raw = repo.run(&["ls-files", "-u", "-z"])?;
    let mut entries_by_path: BTreeMap<String, Vec<IndexStageEntry>> = BTreeMap::new();
    for record in raw.split('\0').filter(|record| !record.is_empty()) {
        let Some((metadata, path)) = record.split_once('\t') else {
            return Err(DevRelayError::Config(format!(
                "unexpected git ls-files -u record: {record:?}"
            )));
        };
        let mut fields = metadata.split_whitespace();
        let mode = fields.next().ok_or_else(|| malformed_record(record))?;
        let oid = fields.next().ok_or_else(|| malformed_record(record))?;
        let stage = fields
            .next()
            .ok_or_else(|| malformed_record(record))?
            .parse::<u8>()
            .map_err(|_| malformed_record(record))?;

        entries_by_path
            .entry(path.replace('\\', "/"))
            .or_default()
            .push(IndexStageEntry {
                stage,
                mode: mode.to_string(),
                oid: oid.to_string(),
            });
    }

    Ok(entries_by_path
        .into_iter()
        .map(|(path, mut stages)| {
            stages.sort_by_key(|entry| entry.stage);
            UnmergedIndexEntry { path, stages }
        })
        .collect())
}

fn capture_conflict_worktree_files(
    repo: &GitRepo,
    unmerged_entries: &[UnmergedIndexEntry],
) -> Result<Vec<ConflictWorktreeFile>> {
    let mut files = Vec::new();
    for entry in unmerged_entries {
        let path = repo.path().join(PathBuf::from(&entry.path));
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        files.push(ConflictWorktreeFile {
            path: entry.path.clone(),
            contents: fs::read(path)?,
        });
    }
    Ok(files)
}

fn malformed_record(record: &str) -> DevRelayError {
    DevRelayError::Config(format!("unexpected git ls-files -u record: {record:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::{Command, Output};

    const _: () = assert!(!REBASE_OPERATION_RECONSTRUCTION_ENABLED);
    const _: () = assert!(REBASE_OPERATION_MIN_TARGET_GIT_VERSION.is_none());

    #[test]
    fn captures_merge_metadata_and_index_stages() {
        let temp = tempfile::tempdir().unwrap();
        let repo = init_repo(temp.path());
        fs::write(temp.path().join("conflict.txt"), "base\n").unwrap();
        git(temp.path(), &["add", "conflict.txt"]);
        git(temp.path(), &["commit", "-m", "base"]);
        git(temp.path(), &["checkout", "-b", "feature"]);
        fs::write(temp.path().join("conflict.txt"), "feature\n").unwrap();
        git(temp.path(), &["commit", "-am", "feature"]);
        git(temp.path(), &["checkout", "main"]);
        fs::write(temp.path().join("conflict.txt"), "main\n").unwrap();
        git(temp.path(), &["commit", "-am", "main"]);

        let merge = git_output(temp.path(), &["merge", "feature"]);
        assert!(!merge.status.success(), "merge should conflict");

        let capsule = capture_operation_capsule(&repo)
            .unwrap()
            .expect("merge conflict should produce an operation capsule");

        assert_eq!(capsule.operation.kind, GitOperationKind::Merge);
        assert_eq!(capsule.operation.operation_oids.len(), 1);
        assert_eq!(capsule.unmerged_entries.len(), 1);
        let entry = &capsule.unmerged_entries[0];
        assert_eq!(entry.path, "conflict.txt");
        assert_eq!(
            entry
                .stages
                .iter()
                .map(|stage| stage.stage)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(entry.stages.iter().all(|stage| stage.mode == "100644"));
        assert!(entry.stages.iter().all(|stage| !stage.oid.is_empty()));
        assert_eq!(capsule.worktree_files.len(), 1);
        let marker_text = String::from_utf8(capsule.worktree_files[0].contents.clone()).unwrap();
        assert!(marker_text.contains("<<<<<<< HEAD"));
        assert!(marker_text.contains("======="));
        assert!(marker_text.contains(">>>>>>> feature"));
    }

    #[test]
    fn applies_unmerged_index_entries_to_target() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let target_path = temp.path().join("target");
        fs::create_dir(&source_path).unwrap();
        let source = init_repo(&source_path);
        fs::write(source_path.join("conflict.txt"), "base\n").unwrap();
        git(&source_path, &["add", "conflict.txt"]);
        git(&source_path, &["commit", "-m", "base"]);
        git(&source_path, &["checkout", "-b", "feature"]);
        fs::write(source_path.join("conflict.txt"), "feature\n").unwrap();
        git(&source_path, &["commit", "-am", "feature"]);
        git(&source_path, &["checkout", "main"]);
        fs::write(source_path.join("conflict.txt"), "main\n").unwrap();
        git(&source_path, &["commit", "-am", "main"]);

        let merge = git_output(&source_path, &["merge", "feature"]);
        assert!(!merge.status.success(), "merge should conflict");
        let capsule = capture_operation_capsule(&source)
            .unwrap()
            .expect("merge conflict should produce an operation capsule");
        git_clone(&source_path, &target_path);
        let target = GitRepo::new(&target_path);

        apply_unmerged_index_entries(&target, &capsule).unwrap();
        restore_conflict_worktree_files(&target, &capsule).unwrap();

        assert_eq!(target.status().unwrap().counts.unmerged, 1);
        assert_eq!(
            capture_unmerged_index_entries(&target).unwrap(),
            capsule.unmerged_entries
        );
        assert_eq!(
            fs::read(target_path.join("conflict.txt")).unwrap(),
            capsule.worktree_files[0].contents
        );
    }

    #[test]
    fn captures_cherry_pick_and_revert_metadata_markers() {
        for (kind, marker) in [
            (GitOperationKind::CherryPick, "CHERRY_PICK_HEAD"),
            (GitOperationKind::Revert, "REVERT_HEAD"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let repo = init_repo(temp.path());
            fs::write(temp.path().join("file.txt"), "base\n").unwrap();
            git(temp.path(), &["add", "file.txt"]);
            git(temp.path(), &["commit", "-m", "base"]);
            let git_dir = repo.git_dir().unwrap();
            fs::write(git_dir.join(marker), "abc123\n").unwrap();
            fs::write(git_dir.join("ORIG_HEAD"), "def456\n").unwrap();

            let capsule = capture_operation_capsule(&repo)
                .unwrap()
                .expect("marker should produce an operation capsule");

            assert_eq!(capsule.operation.kind, kind);
            assert_eq!(capsule.operation.operation_oids, vec!["abc123"]);
            assert_eq!(
                capsule.operation.original_head_oid.as_deref(),
                Some("def456")
            );
            assert!(capsule.unmerged_entries.is_empty());
        }
    }

    #[test]
    fn captures_interactive_rebase_progress_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let repo = init_repo(temp.path());
        fs::write(temp.path().join("file.txt"), "base\n").unwrap();
        git(temp.path(), &["add", "file.txt"]);
        git(temp.path(), &["commit", "-m", "base"]);
        let head = repo.run(&["rev-parse", "HEAD"]).unwrap();
        let onto = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let rebase_dir = repo.git_dir().unwrap().join("rebase-merge");
        fs::create_dir(&rebase_dir).unwrap();
        fs::write(rebase_dir.join("interactive"), "").unwrap();
        fs::write(rebase_dir.join("orig-head"), format!("{head}\n")).unwrap();
        fs::write(rebase_dir.join("onto"), format!("{onto}\n")).unwrap();
        fs::write(rebase_dir.join("head-name"), "refs/heads/main\n").unwrap();
        fs::write(rebase_dir.join("done"), format!("pick {head} base\n")).unwrap();
        fs::write(
            rebase_dir.join("git-rebase-todo"),
            format!("# keep\npick {head} second\nfixup {head} third\n"),
        )
        .unwrap();
        fs::write(rebase_dir.join("msgnum"), "2\n").unwrap();
        fs::write(rebase_dir.join("end"), "3\n").unwrap();

        let capsule = capture_operation_capsule(&repo)
            .unwrap()
            .expect("rebase metadata should produce an operation capsule");
        let progress = capsule
            .operation
            .progress
            .as_ref()
            .expect("rebase should capture progress metadata");

        assert_eq!(capsule.operation.kind, GitOperationKind::RebaseMerge);
        assert_eq!(
            capsule.operation.original_head_oid.as_deref(),
            Some(head.as_str())
        );
        assert!(progress.interactive);
        assert_eq!(progress.original_head_oid.as_deref(), Some(head.as_str()));
        assert_eq!(progress.onto_oid.as_deref(), Some(onto));
        assert_eq!(progress.head_name.as_deref(), Some("refs/heads/main"));
        assert_eq!(
            progress.todo,
            vec![
                "# keep".to_string(),
                format!("pick {head} second"),
                format!("fixup {head} third")
            ]
        );
        assert_eq!(progress.done, vec![format!("pick {head} base")]);
        assert_eq!(
            progress.current_step,
            Some(GitOperationStep {
                current: Some(2),
                total: Some(3),
            })
        );
    }

    #[test]
    fn captures_sequencer_progress_and_keeps_rebase_reconstruction_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let repo = init_repo(temp.path());
        fs::write(temp.path().join("file.txt"), "base\n").unwrap();
        git(temp.path(), &["add", "file.txt"]);
        git(temp.path(), &["commit", "-m", "base"]);
        let head = repo.run(&["rev-parse", "HEAD"]).unwrap();
        let sequencer_dir = repo.git_dir().unwrap().join("sequencer");
        fs::create_dir(&sequencer_dir).unwrap();
        fs::write(sequencer_dir.join("head"), format!("{head}\n")).unwrap();
        fs::write(sequencer_dir.join("done"), format!("pick {head} base\n")).unwrap();
        fs::write(
            sequencer_dir.join("todo"),
            format!("pick {head} second\n# keep\npick {head} third\n"),
        )
        .unwrap();

        let capsule = capture_operation_capsule(&repo)
            .unwrap()
            .expect("sequencer metadata should produce an operation capsule");
        let progress = capsule
            .operation
            .progress
            .as_ref()
            .expect("sequencer should capture progress metadata");

        assert_eq!(capsule.operation.kind, GitOperationKind::Sequencer);
        assert_eq!(
            capsule.operation.original_head_oid.as_deref(),
            Some(head.as_str())
        );
        assert!(!progress.interactive);
        assert_eq!(
            progress.current_step,
            Some(GitOperationStep {
                current: Some(2),
                total: Some(3),
            })
        );
    }

    fn init_repo(root: &Path) -> GitRepo {
        git(root, &["init", "-b", "main"]);
        git(root, &["config", "user.name", "DevRelay Test"]);
        git(
            root,
            &["config", "user.email", "devrelay-test@example.local"],
        );
        GitRepo::new(root)
    }

    fn git(root: &Path, args: &[&str]) {
        let output = git_output(root, args);
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_output(root: &Path, args: &[&str]) -> Output {
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_AUTHOR_NAME", "DevRelay Test")
            .env("GIT_AUTHOR_EMAIL", "devrelay-test@example.local")
            .env("GIT_COMMITTER_NAME", "DevRelay Test")
            .env("GIT_COMMITTER_EMAIL", "devrelay-test@example.local")
            .output()
            .unwrap()
    }

    fn git_clone(source: &Path, target: &Path) {
        let output = Command::new("git")
            .arg("clone")
            .arg(source)
            .arg(target)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git clone failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
