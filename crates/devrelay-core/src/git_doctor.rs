//! Git performance diagnostics for large working trees.

use crate::{DevRelayError, GitRepo, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

const CORE_FSMONITOR: &str = "core.fsmonitor";
const CORE_UNTRACKED_CACHE: &str = "core.untrackedCache";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitPerformanceDoctorReport {
    pub repo: PathBuf,
    pub git_version: String,
    pub parsed_git_version: Option<GitVersion>,
    pub fsmonitor_supported: bool,
    pub fsmonitor_config: Option<String>,
    pub untracked_cache_supported: bool,
    pub untracked_cache_config: Option<String>,
    pub recommendations: Vec<GitPerformanceRecommendation>,
    pub applied_fixes: Vec<GitPerformanceFix>,
    pub skipped_fixes: Vec<GitPerformanceFix>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitPerformanceRecommendation {
    pub code: String,
    pub message: String,
    pub safe_fix: Option<GitPerformanceFix>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitPerformanceFix {
    pub key: String,
    pub value: String,
    pub reason: String,
}

pub fn run_git_performance_doctor(
    repo: &GitRepo,
    fix_safe: bool,
) -> Result<GitPerformanceDoctorReport> {
    repo.run(&["rev-parse", "--git-dir"])?;

    let git_version = repo.run(&["--version"])?;
    let parsed_git_version = parse_git_version(&git_version);
    let fsmonitor_supported = supports_builtin_fsmonitor(parsed_git_version.as_ref());
    let untracked_cache_supported =
        command_succeeds(repo, &["update-index", "--test-untracked-cache"])?;

    let mut applied_fixes = Vec::new();
    let mut skipped_fixes = Vec::new();
    let fsmonitor_config_before = git_config_get(repo, CORE_FSMONITOR)?;
    let untracked_cache_config_before = git_config_get(repo, CORE_UNTRACKED_CACHE)?;

    if fix_safe {
        apply_safe_fix(
            repo,
            fsmonitor_supported,
            fsmonitor_config_before.as_deref(),
            fsmonitor_fix(),
            &mut applied_fixes,
            &mut skipped_fixes,
        )?;
        apply_safe_fix(
            repo,
            untracked_cache_supported,
            untracked_cache_config_before.as_deref(),
            untracked_cache_fix(),
            &mut applied_fixes,
            &mut skipped_fixes,
        )?;
    }

    let fsmonitor_config = git_config_get(repo, CORE_FSMONITOR)?;
    let untracked_cache_config = git_config_get(repo, CORE_UNTRACKED_CACHE)?;
    let recommendations = recommendations(
        fsmonitor_supported,
        fsmonitor_config.as_deref(),
        untracked_cache_supported,
        untracked_cache_config.as_deref(),
    );

    Ok(GitPerformanceDoctorReport {
        repo: repo.path().to_path_buf(),
        git_version,
        parsed_git_version,
        fsmonitor_supported,
        fsmonitor_config,
        untracked_cache_supported,
        untracked_cache_config,
        recommendations,
        applied_fixes,
        skipped_fixes,
    })
}

fn parse_git_version(raw: &str) -> Option<GitVersion> {
    let version = raw
        .split_whitespace()
        .find(|part| part.chars().next().is_some_and(|ch| ch.is_ascii_digit()))?;
    let mut parts = version.split('.');
    Some(GitVersion {
        major: parts.next()?.parse().ok()?,
        minor: parts.next().unwrap_or("0").parse().ok()?,
        patch: parts
            .next()
            .unwrap_or("0")
            .split(|ch: char| !ch.is_ascii_digit())
            .next()
            .unwrap_or("0")
            .parse()
            .ok()?,
    })
}

fn supports_builtin_fsmonitor(version: Option<&GitVersion>) -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
        && version.is_some_and(|version| version.at_least(2, 36, 0))
}

impl GitVersion {
    fn at_least(&self, major: u64, minor: u64, patch: u64) -> bool {
        (self.major, self.minor, self.patch) >= (major, minor, patch)
    }
}

fn recommendations(
    fsmonitor_supported: bool,
    fsmonitor_config: Option<&str>,
    untracked_cache_supported: bool,
    untracked_cache_config: Option<&str>,
) -> Vec<GitPerformanceRecommendation> {
    let mut recommendations = Vec::new();
    if fsmonitor_supported && fsmonitor_config.is_none() {
        recommendations.push(GitPerformanceRecommendation {
            code: "git-fsmonitor-enable".to_string(),
            message: "Enable Git FSMonitor for faster status scans on large working trees."
                .to_string(),
            safe_fix: Some(fsmonitor_fix()),
        });
    }
    if untracked_cache_supported && untracked_cache_config.is_none() {
        recommendations.push(GitPerformanceRecommendation {
            code: "git-untracked-cache-enable".to_string(),
            message: "Enable Git untracked cache for faster untracked path detection.".to_string(),
            safe_fix: Some(untracked_cache_fix()),
        });
    }
    recommendations
}

fn apply_safe_fix(
    repo: &GitRepo,
    supported: bool,
    existing_config: Option<&str>,
    fix: GitPerformanceFix,
    applied_fixes: &mut Vec<GitPerformanceFix>,
    skipped_fixes: &mut Vec<GitPerformanceFix>,
) -> Result<()> {
    if !supported || existing_config.is_some() {
        if existing_config.is_some() {
            skipped_fixes.push(fix);
        }
        return Ok(());
    }
    git_config_set(repo, &fix.key, &fix.value)?;
    applied_fixes.push(fix);
    Ok(())
}

fn fsmonitor_fix() -> GitPerformanceFix {
    GitPerformanceFix {
        key: CORE_FSMONITOR.to_string(),
        value: "true".to_string(),
        reason: "Use Git's built-in FSMonitor when the installed Git supports it.".to_string(),
    }
}

fn untracked_cache_fix() -> GitPerformanceFix {
    GitPerformanceFix {
        key: CORE_UNTRACKED_CACHE.to_string(),
        value: "true".to_string(),
        reason: "Cache untracked-directory metadata for faster status scans.".to_string(),
    }
}

fn command_succeeds(repo: &GitRepo, args: &[&str]) -> Result<bool> {
    let output = git_command(repo, args).output()?;
    Ok(output.status.success())
}

fn git_config_get(repo: &GitRepo, key: &str) -> Result<Option<String>> {
    let output = git_command(repo, &["config", "--local", "--get", key]).output()?;
    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Ok((!value.is_empty()).then_some(value));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    Err(DevRelayError::GitCommand {
        cwd: repo.path().to_path_buf(),
        args: format!("config --local --get {key}"),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

fn git_config_set(repo: &GitRepo, key: &str, value: &str) -> Result<()> {
    let output = git_command(repo, &["config", "--local", key, value]).output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(DevRelayError::GitCommand {
        cwd: repo.path().to_path_buf(),
        args: format!("config --local {key} {value}"),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

fn git_command(repo: &GitRepo, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo.path()).args(args);
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parses_git_versions() {
        assert_eq!(
            parse_git_version("git version 2.44.1"),
            Some(GitVersion {
                major: 2,
                minor: 44,
                patch: 1,
            })
        );
        assert_eq!(
            parse_git_version("git version 2.39.3 (Apple Git-146)"),
            Some(GitVersion {
                major: 2,
                minor: 39,
                patch: 3,
            })
        );
    }

    #[test]
    fn reports_git_performance_recommendations() {
        let temp = tempfile::tempdir().unwrap();
        init_git_repo(temp.path());
        let repo = GitRepo::new(temp.path());

        let report = run_git_performance_doctor(&repo, false).unwrap();

        assert!(report.git_version.starts_with("git version "));
        assert!(report.parsed_git_version.is_some());
        assert!(report.untracked_cache_supported);
        assert_eq!(report.untracked_cache_config, None);
        assert!(
            report
                .recommendations
                .iter()
                .any(|recommendation| recommendation.code == "git-untracked-cache-enable")
        );
    }

    #[test]
    fn fix_safe_applies_only_unset_approved_configs() {
        let temp = tempfile::tempdir().unwrap();
        init_git_repo(temp.path());
        let repo = GitRepo::new(temp.path());

        let report = run_git_performance_doctor(&repo, true).unwrap();

        assert_eq!(report.untracked_cache_config.as_deref(), Some("true"));
        assert!(
            report
                .applied_fixes
                .iter()
                .any(|fix| fix.key == CORE_UNTRACKED_CACHE && fix.value == "true")
        );
    }

    #[test]
    fn fix_safe_does_not_overwrite_existing_user_config() {
        let temp = tempfile::tempdir().unwrap();
        init_git_repo(temp.path());
        let repo = GitRepo::new(temp.path());
        git_config_set(&repo, CORE_UNTRACKED_CACHE, "false").unwrap();

        let report = run_git_performance_doctor(&repo, true).unwrap();

        assert_eq!(report.untracked_cache_config.as_deref(), Some("false"));
        assert!(
            report
                .skipped_fixes
                .iter()
                .any(|fix| fix.key == CORE_UNTRACKED_CACHE)
        );
    }

    fn init_git_repo(root: &Path) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["init", "-b", "main"])
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(root.join("README.md"), "demo\n").unwrap();
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["add", "."])
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["commit", "-m", "base"])
                .env("GIT_AUTHOR_NAME", "DevRelay Test")
                .env("GIT_AUTHOR_EMAIL", "devrelay-test@example.local")
                .env("GIT_COMMITTER_NAME", "DevRelay Test")
                .env("GIT_COMMITTER_EMAIL", "devrelay-test@example.local")
                .status()
                .unwrap()
                .success()
        );
    }
}
