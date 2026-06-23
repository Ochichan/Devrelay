//! In-progress Git operation capture primitives.
//!
//! These structures describe merge-like conflict state without applying it to
//! normal snapshots yet. M12 can use them as the durable operation capsule once
//! conflict round-trip support is exhaustive.

use crate::{DevRelayError, GitRepo, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationCapsule {
    pub operation: GitOperationMetadata,
    pub unmerged_entries: Vec<UnmergedIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitOperationMetadata {
    pub kind: GitOperationKind,
    pub current_head_oid: String,
    pub operation_oids: Vec<String>,
    pub original_head_oid: Option<String>,
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

pub fn capture_operation_capsule(repo: &GitRepo) -> Result<Option<OperationCapsule>> {
    let Some(operation) = capture_operation_metadata(repo)? else {
        return Ok(None);
    };
    Ok(Some(OperationCapsule {
        operation,
        unmerged_entries: capture_unmerged_index_entries(repo)?,
    }))
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

    Ok(Some(GitOperationMetadata {
        kind,
        current_head_oid: repo.run(&["rev-parse", "HEAD"])?,
        operation_oids: marker_oids(&git_dir.join(marker_path))?,
        original_head_oid: optional_oid_file(&git_dir.join("ORIG_HEAD"))?,
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

fn optional_oid_file(path: &std::path::Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(fs::read_to_string(path)?
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string))
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

fn malformed_record(record: &str) -> DevRelayError {
    DevRelayError::Config(format!("unexpected git ls-files -u record: {record:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::{Command, Output};

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
}
