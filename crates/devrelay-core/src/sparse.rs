//! Sparse checkout and partial clone inspection helpers.

use crate::{DevRelayError, GitRepo, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
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
