//! Sparse checkout and partial clone inspection helpers.

use crate::{DevRelayError, GitRepo, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SparseCheckoutReport {
    pub repo: PathBuf,
    pub sparse_checkout_enabled: bool,
    pub cone_mode: bool,
    pub sparse_patterns: Vec<String>,
    pub partial_clone: PartialCloneState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PartialCloneState {
    pub enabled: bool,
    pub filter: Option<String>,
    pub promisor_remotes: Vec<String>,
    pub extension_remote: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlobAvailabilityReport {
    pub repo: PathBuf,
    pub treeish: String,
    pub checked_blobs: usize,
    pub missing_blobs: Vec<String>,
}

pub fn inspect_sparse_checkout(repo: &GitRepo) -> Result<SparseCheckoutReport> {
    let sparse_checkout_enabled = git_bool_config(repo, "core.sparseCheckout")?;
    let cone_mode = git_bool_config(repo, "core.sparseCheckoutCone")?;
    let sparse_patterns = read_sparse_patterns(repo)?;
    let partial_clone = inspect_partial_clone(repo)?;

    Ok(SparseCheckoutReport {
        repo: repo.path().to_path_buf(),
        sparse_checkout_enabled,
        cone_mode,
        sparse_patterns,
        partial_clone,
    })
}

pub fn fetch_missing_blobs_on_demand(
    repo: &GitRepo,
    treeish: &str,
) -> Result<BlobAvailabilityReport> {
    let blob_oids = blob_oids_in_tree(repo, treeish)?;
    let mut missing_blobs = Vec::new();
    for oid in &blob_oids {
        if repo
            .run(&["cat-file", "-e", &format!("{oid}^{{blob}}")])
            .is_err()
        {
            missing_blobs.push(oid.clone());
        }
    }

    let report = BlobAvailabilityReport {
        repo: repo.path().to_path_buf(),
        treeish: treeish.to_string(),
        checked_blobs: blob_oids.len(),
        missing_blobs,
    };
    if !report.missing_blobs.is_empty() {
        return Err(DevRelayError::MissingSourceObject(format!(
            "missing {} Git blobs required by {}: {}",
            report.missing_blobs.len(),
            treeish,
            report.missing_blobs.join(", ")
        )));
    }
    Ok(report)
}

fn blob_oids_in_tree(repo: &GitRepo, treeish: &str) -> Result<Vec<String>> {
    let raw = repo.run(&["ls-tree", "-r", "-z", treeish])?;
    let mut oids = BTreeSet::new();
    for record in raw.split('\0').filter(|record| !record.is_empty()) {
        let Some((metadata, _path)) = record.split_once('\t') else {
            return Err(DevRelayError::Config(format!(
                "unexpected git ls-tree record: {record:?}"
            )));
        };
        let mut fields = metadata.split_whitespace();
        let _mode = fields.next().unwrap_or_default();
        let object_type = fields.next().unwrap_or_default();
        let oid = fields.next().unwrap_or_default();
        if object_type == "blob" && !oid.is_empty() {
            oids.insert(oid.to_string());
        }
    }
    Ok(oids.into_iter().collect())
}

fn inspect_partial_clone(repo: &GitRepo) -> Result<PartialCloneState> {
    let extension_remote = git_optional_config(repo, "extensions.partialClone")?;
    let remote_filters = remote_partial_clone_filters(repo)?;
    let mut promisor_remotes = remote_promisor_remotes(repo)?;
    if let Some(remote) = extension_remote.as_ref()
        && !promisor_remotes.contains(remote)
    {
        promisor_remotes.push(remote.clone());
        promisor_remotes.sort();
    }

    let filter = extension_remote
        .as_ref()
        .and_then(|remote| remote_filters.get(remote))
        .or_else(|| {
            promisor_remotes
                .iter()
                .find_map(|remote| remote_filters.get(remote))
        })
        .or_else(|| remote_filters.values().next())
        .cloned();
    let enabled = extension_remote.is_some() || !promisor_remotes.is_empty() || filter.is_some();

    Ok(PartialCloneState {
        enabled,
        filter,
        promisor_remotes,
        extension_remote,
    })
}

fn read_sparse_patterns(repo: &GitRepo) -> Result<Vec<String>> {
    let sparse_checkout = repo.git_dir()?.join("info").join("sparse-checkout");
    let raw = match fs::read_to_string(sparse_checkout) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    Ok(raw
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect())
}

fn git_bool_config(repo: &GitRepo, key: &str) -> Result<bool> {
    match repo.run(&["config", "--bool", "--get", key]) {
        Ok(raw) => Ok(raw.trim() == "true"),
        Err(err) if is_missing_config_value(&err) => Ok(false),
        Err(err) => Err(err),
    }
}

fn git_optional_config(repo: &GitRepo, key: &str) -> Result<Option<String>> {
    match repo.run(&["config", "--get", key]) {
        Ok(raw) => {
            let value = raw.trim();
            Ok((!value.is_empty()).then(|| value.to_string()))
        }
        Err(err) if is_missing_config_value(&err) => Ok(None),
        Err(err) => Err(err),
    }
}

fn remote_promisor_remotes(repo: &GitRepo) -> Result<Vec<String>> {
    let raw = git_optional_regexp(
        repo,
        &[
            "config",
            "--bool",
            "--get-regexp",
            "^remote\\..*\\.promisor$",
        ],
    )?;
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };

    let mut remotes = Vec::new();
    for (key, value) in config_records(&raw)? {
        if parse_git_bool(&value)
            && let Some(remote) = remote_config_name(&key, ".promisor")
        {
            remotes.push(remote);
        }
    }
    remotes.sort();
    remotes.dedup();
    Ok(remotes)
}

fn remote_partial_clone_filters(repo: &GitRepo) -> Result<BTreeMap<String, String>> {
    let raw = git_optional_regexp(
        repo,
        &[
            "config",
            "--get-regexp",
            "^remote\\..*\\.partialclonefilter$",
        ],
    )?;
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };

    let mut filters = BTreeMap::new();
    for (key, value) in config_records(&raw)? {
        if let Some(remote) = remote_config_name(&key, ".partialclonefilter") {
            filters.insert(remote, value);
        }
    }
    Ok(filters)
}

fn git_optional_regexp(repo: &GitRepo, args: &[&str]) -> Result<Option<String>> {
    match repo.run(args) {
        Ok(raw) => Ok(Some(raw)),
        Err(err) if is_missing_config_value(&err) => Ok(None),
        Err(err) => Err(err),
    }
}

fn config_records(raw: &str) -> Result<Vec<(String, String)>> {
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let Some((key, value)) = line.split_once(' ') else {
                return Err(DevRelayError::Config(format!(
                    "unexpected git config record: {line:?}"
                )));
            };
            Ok((key.to_string(), value.to_string()))
        })
        .collect()
}

fn remote_config_name(key: &str, suffix: &str) -> Option<String> {
    key.strip_prefix("remote.")
        .and_then(|rest| rest.strip_suffix(suffix))
        .map(str::to_string)
}

fn parse_git_bool(value: &str) -> bool {
    matches!(value.trim(), "true" | "yes" | "on" | "1")
}

fn is_missing_config_value(err: &DevRelayError) -> bool {
    matches!(err, DevRelayError::GitCommand { stderr, .. } if stderr.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    #[test]
    fn detects_sparse_checkout_definition() {
        let temp = tempfile::tempdir().unwrap();
        let repo = init_repo(temp.path());
        git(temp.path(), &["config", "core.sparseCheckout", "true"]);
        git(temp.path(), &["config", "core.sparseCheckoutCone", "true"]);
        fs::create_dir_all(repo.git_dir().unwrap().join("info")).unwrap();
        fs::write(
            repo.git_dir().unwrap().join("info/sparse-checkout"),
            "/src/\n!/src/generated/\n\n# keep docs\n/docs/\n",
        )
        .unwrap();

        let report = inspect_sparse_checkout(&repo).unwrap();

        assert!(report.sparse_checkout_enabled);
        assert!(report.cone_mode);
        assert_eq!(
            report.sparse_patterns,
            vec!["/src/", "!/src/generated/", "# keep docs", "/docs/"]
        );
        assert!(!report.partial_clone.enabled);
    }

    #[test]
    fn detects_partial_clone_config() {
        let temp = tempfile::tempdir().unwrap();
        let repo = init_repo(temp.path());
        git(
            temp.path(),
            &["config", "extensions.partialClone", "origin"],
        );
        git(temp.path(), &["config", "remote.origin.promisor", "true"]);
        git(
            temp.path(),
            &["config", "remote.origin.partialclonefilter", "blob:none"],
        );

        let report = inspect_sparse_checkout(&repo).unwrap();

        assert!(!report.sparse_checkout_enabled);
        assert!(report.partial_clone.enabled);
        assert_eq!(
            report.partial_clone.extension_remote.as_deref(),
            Some("origin")
        );
        assert_eq!(report.partial_clone.promisor_remotes, vec!["origin"]);
        assert_eq!(report.partial_clone.filter.as_deref(), Some("blob:none"));
    }

    #[test]
    fn fetch_missing_blobs_on_demand_checks_tree_blobs() {
        let temp = tempfile::tempdir().unwrap();
        let repo = init_repo(temp.path());
        fs::write(temp.path().join("tracked.txt"), "present\n").unwrap();
        git(temp.path(), &["add", "tracked.txt"]);
        git(temp.path(), &["commit", "-m", "base"]);

        let report = fetch_missing_blobs_on_demand(&repo, "HEAD").unwrap();

        assert_eq!(report.checked_blobs, 1);
        assert!(report.missing_blobs.is_empty());
    }

    #[test]
    fn fetch_missing_blobs_on_demand_reports_missing_blob() {
        let temp = tempfile::tempdir().unwrap();
        let repo = init_repo(temp.path());
        fs::write(temp.path().join("tracked.txt"), "missing\n").unwrap();
        git(temp.path(), &["add", "tracked.txt"]);
        git(temp.path(), &["commit", "-m", "base"]);
        let blob_oid = repo.run(&["rev-parse", "HEAD:tracked.txt"]).unwrap();
        let blob_path = repo
            .git_dir()
            .unwrap()
            .join("objects")
            .join(&blob_oid[..2])
            .join(&blob_oid[2..]);
        fs::remove_file(blob_path).unwrap();

        let err = fetch_missing_blobs_on_demand(&repo, "HEAD").unwrap_err();

        assert!(matches!(err, DevRelayError::MissingSourceObject(_)));
        assert!(err.to_string().contains(&blob_oid));
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
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_AUTHOR_NAME", "DevRelay Test")
            .env("GIT_AUTHOR_EMAIL", "devrelay-test@example.local")
            .env("GIT_COMMITTER_NAME", "DevRelay Test")
            .env("GIT_COMMITTER_EMAIL", "devrelay-test@example.local")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
