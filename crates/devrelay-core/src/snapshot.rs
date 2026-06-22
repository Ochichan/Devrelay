use crate::error::{DevRelayError, Result};
use crate::git::{GitRepo, GitStatus, StatusCounts};
use crate::manifest::Manifest;
use crate::policy::{ClassifiedPath, PathDecision, classify_untracked_paths};
use serde::{Deserialize, Serialize};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotMetadata {
    pub schema_version: u32,
    pub snapshot_id: String,
    pub project_id: String,
    pub project_name: String,
    pub branch: Option<String>,
    pub head_oid: String,
    pub index_tree_oid: String,
    pub index_commit_oid: String,
    pub work_tree_oid: String,
    pub work_commit_oid: String,
    pub source_status: StatusCounts,
    pub included_untracked: Vec<String>,
    pub excluded: Vec<ClassifiedPath>,
    pub state_hash: String,
    pub created_at_unix_seconds: u64,
}

impl SnapshotMetadata {
    pub fn index_ref(&self) -> String {
        format!("refs/devrelay/snapshots/{}/index", self.snapshot_id)
    }

    pub fn work_ref(&self) -> String {
        format!("refs/devrelay/snapshots/{}/work", self.snapshot_id)
    }
}

pub fn create_snapshot(repo: &GitRepo, manifest: &Manifest) -> Result<SnapshotMetadata> {
    let status = repo.status()?;
    let classified = classify_untracked_paths(repo.path(), manifest, status.untracked_paths())?;
    let included_untracked = classified
        .iter()
        .filter(|item| item.decision == PathDecision::Include)
        .map(|item| item.path.clone())
        .collect::<Vec<_>>();
    let excluded = classified
        .into_iter()
        .filter(|item| item.decision == PathDecision::Exclude)
        .collect::<Vec<_>>();

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
        schema_version: 1,
        snapshot_id,
        project_id: manifest.project_id.clone(),
        project_name: manifest.name.clone(),
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
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(metadata)?)?;
    Ok(())
}

pub fn read_snapshot_file(path: impl AsRef<Path>) -> Result<SnapshotMetadata> {
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn apply_snapshot(
    target: &GitRepo,
    source: &GitRepo,
    snapshot: &SnapshotMetadata,
) -> Result<()> {
    let target_status = target.status()?;
    if !target_status.is_clean() {
        return Err(DevRelayError::TargetDirty(target_status.short_summary()));
    }

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

pub fn verify_snapshot(repo: &GitRepo, snapshot: &SnapshotMetadata) -> Result<()> {
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

    Ok(())
}

fn fetch_snapshot_refs(
    target: &GitRepo,
    source: &GitRepo,
    snapshot: &SnapshotMetadata,
) -> Result<()> {
    let source_path = source.path().as_os_str().to_os_string();
    target.run_with_env(
        [
            OsString::from("fetch"),
            source_path,
            OsString::from(format!("{}:{}", snapshot.index_ref(), snapshot.index_ref())),
            OsString::from(format!("{}:{}", snapshot.work_ref(), snapshot.work_ref())),
        ],
        &[],
    )?;
    Ok(())
}

fn write_work_tree(repo: &GitRepo, included_untracked: &[String]) -> Result<String> {
    let temp_dir = tempfile::tempdir()?;
    let temp_index = temp_dir.path().join("index");
    copy_current_index(repo, &temp_index)?;

    git_with_index(repo, &temp_index, ["add", "-u", "--"])?;
    if !included_untracked.is_empty() {
        let mut args = vec![OsString::from("add"), OsString::from("--")];
        args.extend(included_untracked.iter().map(OsString::from));
        repo.run_with_env(args, &[("GIT_INDEX_FILE", temp_index.as_os_str())])?;
    }
    git_with_index(repo, &temp_index, ["write-tree"])
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
    format!("s_{}", &digest.to_hex()[..24])
}

fn calculate_state_hash(metadata: &SnapshotMetadata) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(metadata.project_id.as_bytes());
    hasher.update(metadata.head_oid.as_bytes());
    hasher.update(metadata.index_tree_oid.as_bytes());
    hasher.update(metadata.work_tree_oid.as_bytes());
    if let Some(branch) = &metadata.branch {
        hasher.update(branch.as_bytes());
    }
    for path in &metadata.included_untracked {
        hasher.update(path.as_bytes());
        hasher.update(&[0]);
    }
    for item in &metadata.excluded {
        hasher.update(item.path.as_bytes());
        hasher.update(item.reason.as_bytes());
        hasher.update(&[0]);
    }
    hasher.finalize().to_hex().to_string()
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
    use crate::manifest::Manifest;
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

    #[test]
    fn creates_and_applies_snapshot_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("source");
        let target_path = temp.path().join("target");
        fs::create_dir(&source_path).unwrap();
        let source = GitRepo::new(&source_path);
        source.run(&["init", "-b", "main"]).unwrap();
        source
            .run(&["config", "user.name", "DevRelay Test"])
            .unwrap();
        source
            .run(&["config", "user.email", "devrelay-test@example.local"])
            .unwrap();
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
        fs::create_dir(&source_path).unwrap();
        let source = GitRepo::new(&source_path);
        source.run(&["init", "-b", "main"]).unwrap();
        source
            .run(&["config", "user.name", "DevRelay Test"])
            .unwrap();
        source
            .run(&["config", "user.email", "devrelay-test@example.local"])
            .unwrap();
        fs::write(source_path.join("tracked.txt"), "base\n").unwrap();
        source.run(&["add", "."]).unwrap();
        source.run(&["commit", "-m", "base"]).unwrap();
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
    }
}
