//! Git submodule inspection and clean-state restoration helpers.

use crate::{DevRelayError, GitRepo, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubmoduleReport {
    pub repo: PathBuf,
    pub submodules: Vec<SubmoduleState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubmoduleState {
    pub name: String,
    pub path: String,
    pub url: Option<String>,
    pub recorded_commit: Option<String>,
    pub worktree_commit: Option<String>,
    pub status: SubmoduleStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SubmoduleStatus {
    Clean,
    Missing,
    HeadMismatch,
    Dirty,
}

#[derive(Debug, Default)]
struct SubmoduleConfig {
    path: Option<String>,
    url: Option<String>,
}

pub fn inspect_submodules(repo: &GitRepo) -> Result<SubmoduleReport> {
    let mut submodules = Vec::new();
    for (name, config) in submodule_configs(repo)? {
        if let Some(path) = config.path {
            submodules.push(inspect_submodule(repo, name, path, config.url)?);
        }
    }
    submodules.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(SubmoduleReport {
        repo: repo.path().to_path_buf(),
        submodules,
    })
}

pub fn restore_clean_submodule_recorded_commit(repo: &GitRepo, path: &str) -> Result<()> {
    let Some(state) = inspect_submodules(repo)?
        .submodules
        .into_iter()
        .find(|state| state.path == path)
    else {
        return Err(DevRelayError::Config(format!(
            "submodule path {path:?} is not declared in .gitmodules"
        )));
    };

    if state.status == SubmoduleStatus::Dirty {
        return Err(DevRelayError::UnsupportedRepositoryState(format!(
            "submodule {} has dirty worktree changes; commit, stash, or discard them before restore",
            state.path
        )));
    }
    let Some(recorded_commit) = state.recorded_commit else {
        return Err(DevRelayError::UnsupportedRepositoryState(format!(
            "submodule {} has no recorded gitlink commit",
            state.path
        )));
    };
    if state.worktree_commit.is_none() {
        return Err(DevRelayError::UnsupportedRepositoryState(format!(
            "submodule {} is not initialized",
            state.path
        )));
    }

    GitRepo::new(repo.path().join(PathBuf::from(&state.path))).run(&[
        "checkout",
        "--detach",
        &recorded_commit,
    ])?;
    Ok(())
}

fn submodule_configs(repo: &GitRepo) -> Result<BTreeMap<String, SubmoduleConfig>> {
    if !repo.path().join(".gitmodules").exists() {
        return Ok(BTreeMap::new());
    }
    let raw = match repo.run(&[
        "config",
        "--file",
        ".gitmodules",
        "--get-regexp",
        "^submodule\\..*\\.(path|url)$",
    ]) {
        Ok(raw) => raw,
        Err(DevRelayError::GitCommand { .. }) => return Ok(BTreeMap::new()),
        Err(err) => return Err(err),
    };

    let mut configs = BTreeMap::<String, SubmoduleConfig>::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let Some((key, value)) = line.split_once(' ') else {
            return Err(DevRelayError::Config(format!(
                "unexpected .gitmodules config record: {line:?}"
            )));
        };
        let Some(rest) = key.strip_prefix("submodule.") else {
            continue;
        };
        let Some((name, field)) = rest.rsplit_once('.') else {
            continue;
        };
        let config = configs.entry(name.to_string()).or_default();
        match field {
            "path" => config.path = Some(value.to_string()),
            "url" => config.url = Some(value.to_string()),
            _ => {}
        }
    }
    Ok(configs)
}

fn inspect_submodule(
    repo: &GitRepo,
    name: String,
    path: String,
    url: Option<String>,
) -> Result<SubmoduleState> {
    let recorded_commit = recorded_submodule_commit(repo, &path)?;
    let submodule_repo = GitRepo::new(repo.path().join(PathBuf::from(&path)));
    let worktree_commit = submodule_repo.run(&["rev-parse", "HEAD"]).ok();
    let dirty = worktree_commit
        .as_ref()
        .map(|_| submodule_repo.run(&["status", "--porcelain"]))
        .transpose()?
        .is_some_and(|status| !status.trim().is_empty());
    let status = match (&recorded_commit, &worktree_commit, dirty) {
        (_, None, _) => SubmoduleStatus::Missing,
        (_, Some(_), true) => SubmoduleStatus::Dirty,
        (Some(recorded), Some(worktree), false) if recorded != worktree => {
            SubmoduleStatus::HeadMismatch
        }
        _ => SubmoduleStatus::Clean,
    };

    Ok(SubmoduleState {
        name,
        path,
        url,
        recorded_commit,
        worktree_commit,
        status,
    })
}

fn recorded_submodule_commit(repo: &GitRepo, path: &str) -> Result<Option<String>> {
    let raw = match repo.run(&["ls-files", "-s", "--", path]) {
        Ok(raw) => raw,
        Err(DevRelayError::GitCommand { .. }) => return Ok(None),
        Err(err) => return Err(err),
    };
    let Some((metadata, _)) = raw.split_once('\t') else {
        return Ok(None);
    };
    let mut fields = metadata.split_whitespace();
    let mode = fields.next().unwrap_or_default();
    let oid = fields.next().unwrap_or_default();
    if mode == "160000" && !oid.is_empty() {
        Ok(Some(oid.to_string()))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::process::{Command, Output};

    #[test]
    fn detects_clean_dirty_and_restores_recorded_submodule_commit() {
        let temp = tempfile::tempdir().unwrap();
        let child_path = temp.path().join("child");
        let parent_path = temp.path().join("parent");
        fs::create_dir_all(&child_path).unwrap();
        fs::create_dir_all(&parent_path).unwrap();
        let child = init_repo(&child_path);
        fs::write(child_path.join("lib.txt"), "v1\n").unwrap();
        git(&child_path, &["add", "lib.txt"]);
        git(&child_path, &["commit", "-m", "v1"]);
        let child_v1 = child.run(&["rev-parse", "HEAD"]).unwrap();

        let parent = init_repo(&parent_path);
        git_allow_file(
            &parent_path,
            &[
                "submodule",
                "add",
                child_path.to_str().unwrap(),
                "deps/child",
            ],
        );
        git(&parent_path, &["commit", "-am", "add submodule"]);

        let report = inspect_submodules(&parent).unwrap();
        assert_eq!(report.submodules.len(), 1);
        let state = &report.submodules[0];
        assert_eq!(state.path, "deps/child");
        assert_eq!(state.recorded_commit.as_deref(), Some(child_v1.as_str()));
        assert_eq!(state.worktree_commit.as_deref(), Some(child_v1.as_str()));
        assert_eq!(state.status, SubmoduleStatus::Clean);

        fs::write(child_path.join("lib.txt"), "v2\n").unwrap();
        git(&child_path, &["commit", "-am", "v2"]);
        let child_v2 = child.run(&["rev-parse", "HEAD"]).unwrap();
        git_allow_file(&parent_path.join("deps/child"), &["fetch"]);
        git(
            &parent_path.join("deps/child"),
            &["checkout", "--detach", &child_v2],
        );
        let report = inspect_submodules(&parent).unwrap();
        assert_eq!(report.submodules[0].status, SubmoduleStatus::HeadMismatch);

        restore_clean_submodule_recorded_commit(&parent, "deps/child").unwrap();
        let report = inspect_submodules(&parent).unwrap();
        assert_eq!(report.submodules[0].status, SubmoduleStatus::Clean);
        assert_eq!(
            report.submodules[0].worktree_commit.as_deref(),
            Some(child_v1.as_str())
        );

        fs::write(parent_path.join("deps/child/lib.txt"), "dirty\n").unwrap();
        let report = inspect_submodules(&parent).unwrap();
        assert_eq!(report.submodules[0].status, SubmoduleStatus::Dirty);
        let err = restore_clean_submodule_recorded_commit(&parent, "deps/child").unwrap_err();
        assert!(matches!(err, DevRelayError::UnsupportedRepositoryState(_)));
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
        let output = git_output(root, args, false);
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_allow_file(root: &Path, args: &[&str]) {
        let output = git_output(root, args, true);
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_output(root: &Path, args: &[&str], allow_file: bool) -> Output {
        let mut command = Command::new("git");
        command.arg("-C").arg(root).args(args);
        if allow_file {
            command.env("GIT_ALLOW_PROTOCOL", "file");
        }
        command
            .env("GIT_AUTHOR_NAME", "DevRelay Test")
            .env("GIT_AUTHOR_EMAIL", "devrelay-test@example.local")
            .env("GIT_COMMITTER_NAME", "DevRelay Test")
            .env("GIT_COMMITTER_EMAIL", "devrelay-test@example.local")
            .output()
            .unwrap()
    }
}
