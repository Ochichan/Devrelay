//! Line-ending diagnostics for cross-platform handoff safety.

use crate::{DevRelayError, GitRepo, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

const CORE_AUTOCRLF: &str = "core.autocrlf";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineEndingDoctorReport {
    pub repo: PathBuf,
    pub target_platform_key: String,
    pub gitattributes_path: PathBuf,
    pub gitattributes_present: bool,
    pub gitattributes_policy_lines: Vec<String>,
    pub core_autocrlf: Option<String>,
    pub tracked_file_count: usize,
    pub semantic_hash_mismatches: Vec<LineEndingHashMismatch>,
    pub warnings: Vec<LineEndingWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineEndingHashMismatch {
    pub path: String,
    pub index_oid: String,
    pub semantic_oid: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LineEndingWarning {
    pub code: LineEndingWarningCode,
    pub message: String,
    pub safe_actions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum LineEndingWarningCode {
    MissingGitattributesPolicy,
    ConflictingAutocrlf,
    RiskyTargetLineEndingConfig,
    SemanticHashMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GitattributesPolicy {
    path: PathBuf,
    present: bool,
    policy_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrackedFile {
    path: String,
    index_oid: String,
}

pub fn run_line_ending_doctor(
    repo: &GitRepo,
    target_platform_key: &str,
) -> Result<LineEndingDoctorReport> {
    repo.run(&["rev-parse", "--git-dir"])?;

    let gitattributes = read_gitattributes_policy(repo)?;
    let core_autocrlf = git_config_get(repo, CORE_AUTOCRLF)?;
    let tracked_files = tracked_regular_files(repo)?;
    let semantic_hash_mismatches = semantic_hash_mismatches(repo, &tracked_files)?;
    let warnings = line_ending_warnings(
        target_platform_key,
        &gitattributes,
        core_autocrlf.as_deref(),
        &semantic_hash_mismatches,
    );

    Ok(LineEndingDoctorReport {
        repo: repo.path().to_path_buf(),
        target_platform_key: target_platform_key.to_string(),
        gitattributes_path: gitattributes.path,
        gitattributes_present: gitattributes.present,
        gitattributes_policy_lines: gitattributes.policy_lines,
        core_autocrlf,
        tracked_file_count: tracked_files.len(),
        semantic_hash_mismatches,
        warnings,
    })
}

fn read_gitattributes_policy(repo: &GitRepo) -> Result<GitattributesPolicy> {
    let path = repo.path().join(".gitattributes");
    if !path.exists() {
        return Ok(GitattributesPolicy {
            path,
            present: false,
            policy_lines: Vec::new(),
        });
    }

    let raw = std::fs::read(&path)?;
    let text = String::from_utf8_lossy(&raw);
    let policy_lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|line| line_has_line_ending_policy(line))
        .map(str::to_string)
        .collect();

    Ok(GitattributesPolicy {
        path,
        present: true,
        policy_lines,
    })
}

fn line_has_line_ending_policy(line: &str) -> bool {
    line.split_whitespace()
        .skip(1)
        .any(is_line_ending_attribute)
}

fn is_line_ending_attribute(attribute: &str) -> bool {
    attribute == "text"
        || attribute == "-text"
        || attribute.starts_with("text=")
        || attribute.starts_with("eol=")
        || attribute.starts_with("working-tree-encoding=")
}

fn tracked_regular_files(repo: &GitRepo) -> Result<Vec<TrackedFile>> {
    let raw = repo.run(&["ls-files", "-s", "-z"])?;
    let mut files = Vec::new();
    for record in raw.split('\0').filter(|record| !record.is_empty()) {
        let Some((metadata, path)) = record.split_once('\t') else {
            return Err(DevRelayError::Config(format!(
                "unexpected git ls-files record: {record:?}"
            )));
        };
        let mut fields = metadata.split_whitespace();
        let mode = fields.next().unwrap_or_default();
        let index_oid = fields.next().unwrap_or_default();
        let stage = fields.next().unwrap_or_default();
        if mode.starts_with("100") && stage == "0" && !index_oid.is_empty() {
            files.push(TrackedFile {
                path: normalize_repo_path(path),
                index_oid: index_oid.to_string(),
            });
        }
    }
    Ok(files)
}

fn semantic_hash_mismatches(
    repo: &GitRepo,
    tracked_files: &[TrackedFile],
) -> Result<Vec<LineEndingHashMismatch>> {
    let mut mismatches = Vec::new();
    for file in tracked_files {
        match semantic_hash_for_path(repo, &file.path)? {
            SemanticHashResult::Oid(semantic_oid) => {
                if semantic_oid != file.index_oid {
                    mismatches.push(LineEndingHashMismatch {
                        path: file.path.clone(),
                        index_oid: file.index_oid.clone(),
                        semantic_oid: Some(semantic_oid),
                        message: "Working tree content differs from the indexed Git-cleaned blob."
                            .to_string(),
                    });
                }
            }
            SemanticHashResult::Unavailable(message) => {
                mismatches.push(LineEndingHashMismatch {
                    path: file.path.clone(),
                    index_oid: file.index_oid.clone(),
                    semantic_oid: None,
                    message,
                });
            }
        }
    }
    Ok(mismatches)
}

fn semantic_hash_for_path(repo: &GitRepo, path: &str) -> Result<SemanticHashResult> {
    let output = git_command(repo)
        .arg("hash-object")
        .arg(format!("--path={path}"))
        .arg("--")
        .arg(path)
        .output()?;
    if output.status.success() {
        let oid = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Ok(SemanticHashResult::Oid(oid));
    }
    Ok(SemanticHashResult::Unavailable(
        String::from_utf8_lossy(&output.stderr).trim().to_string(),
    ))
}

fn git_config_get(repo: &GitRepo, key: &str) -> Result<Option<String>> {
    let output = git_command(repo).args(["config", "--get", key]).output()?;
    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Ok((!value.is_empty()).then_some(value));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    Err(DevRelayError::GitCommand {
        cwd: repo.path().to_path_buf(),
        args: format!("config --get {key}"),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

fn git_command(repo: &GitRepo) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo.path());
    command
}

fn line_ending_warnings(
    target_platform_key: &str,
    gitattributes: &GitattributesPolicy,
    core_autocrlf: Option<&str>,
    semantic_hash_mismatches: &[LineEndingHashMismatch],
) -> Vec<LineEndingWarning> {
    let mut warnings = Vec::new();
    let has_policy = !gitattributes.policy_lines.is_empty();

    if !has_policy {
        warnings.push(warning(
            LineEndingWarningCode::MissingGitattributesPolicy,
            "Repository has no .gitattributes line-ending policy.".to_string(),
        ));
    }

    if has_policy && core_autocrlf.is_some_and(is_enabled_autocrlf) {
        warnings.push(warning(
            LineEndingWarningCode::ConflictingAutocrlf,
            "Effective core.autocrlf is enabled while .gitattributes also owns line endings."
                .to_string(),
        ));
    }

    if target_line_ending_config_is_risky(target_platform_key, has_policy, core_autocrlf) {
        warnings.push(warning(
            LineEndingWarningCode::RiskyTargetLineEndingConfig,
            format!(
                "Target platform {target_platform_key} can produce line-ending drift with the current repository policy."
            ),
        ));
    }

    if !semantic_hash_mismatches.is_empty() {
        warnings.push(warning(
            LineEndingWarningCode::SemanticHashMismatch,
            format!(
                "{} tracked file(s) do not match their Git-cleaned index blobs.",
                semantic_hash_mismatches.len()
            ),
        ));
    }

    warnings
}

fn is_enabled_autocrlf(value: &str) -> bool {
    matches!(value.to_ascii_lowercase().as_str(), "true" | "input")
}

fn target_line_ending_config_is_risky(
    target_platform_key: &str,
    has_policy: bool,
    core_autocrlf: Option<&str>,
) -> bool {
    if target_platform_key.starts_with("windows-native-") {
        return !has_policy
            || core_autocrlf.is_some_and(|value| value.eq_ignore_ascii_case("input"));
    }
    core_autocrlf.is_some_and(|value| value.eq_ignore_ascii_case("true"))
}

fn warning(code: LineEndingWarningCode, message: String) -> LineEndingWarning {
    LineEndingWarning {
        code,
        message,
        safe_actions: safe_actions_for(code),
    }
}

fn safe_actions_for(code: LineEndingWarningCode) -> Vec<String> {
    match code {
        LineEndingWarningCode::MissingGitattributesPolicy => vec![
            "Add `* text=auto` or explicit `eol=lf`/`eol=crlf` rules to .gitattributes."
                .to_string(),
            "Run `git add --renormalize .` after choosing the policy.".to_string(),
        ],
        LineEndingWarningCode::ConflictingAutocrlf => vec![
            "Set repository-local `core.autocrlf=false` when .gitattributes owns line endings."
                .to_string(),
            "Keep line-ending behavior in .gitattributes so every platform shares one policy."
                .to_string(),
        ],
        LineEndingWarningCode::RiskyTargetLineEndingConfig => vec![
            "Add a repository line-ending policy before handing this workspace to the target platform."
                .to_string(),
            "Avoid relying on per-machine Git config for cross-platform line endings.".to_string(),
        ],
        LineEndingWarningCode::SemanticHashMismatch => vec![
            "Inspect the listed files and stage, revert, or renormalize them before handoff."
                .to_string(),
            "Use `git hash-object --path=<path> -- <path>` to verify Git-cleaned content manually."
                .to_string(),
        ],
    }
}

fn normalize_repo_path(path: &str) -> String {
    path.replace('\\', "/")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SemanticHashResult {
    Oid(String),
    Unavailable(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn detects_missing_policy_and_risky_windows_target() {
        let temp = tempfile::tempdir().unwrap();
        init_git_repo(temp.path());

        let report =
            run_line_ending_doctor(&GitRepo::new(temp.path()), "windows-native-x86_64").unwrap();

        assert!(!report.gitattributes_present);
        assert_eq!(report.gitattributes_policy_lines, Vec::<String>::new());
        assert!(report.tracked_file_count >= 1);
        assert_has_warning(
            &report.warnings,
            LineEndingWarningCode::MissingGitattributesPolicy,
        );
        assert_has_warning(
            &report.warnings,
            LineEndingWarningCode::RiskyTargetLineEndingConfig,
        );
    }

    #[test]
    fn detects_conflicting_autocrlf_with_gitattributes_policy() {
        let temp = tempfile::tempdir().unwrap();
        init_git_repo(temp.path());
        std::fs::write(
            temp.path().join(".gitattributes"),
            "* text=auto\n*.sh text eol=lf\n",
        )
        .unwrap();
        git(temp.path(), &["add", ".gitattributes"]);
        git(temp.path(), &["commit", "-m", "line ending policy"]);
        git(temp.path(), &["config", "core.autocrlf", "true"]);

        let report = run_line_ending_doctor(&GitRepo::new(temp.path()), "darwin-arm64").unwrap();

        assert!(report.gitattributes_present);
        assert_eq!(report.core_autocrlf.as_deref(), Some("true"));
        assert_has_warning(&report.warnings, LineEndingWarningCode::ConflictingAutocrlf);
    }

    #[test]
    fn verifies_working_tree_by_git_semantic_hash() {
        let temp = tempfile::tempdir().unwrap();
        init_git_repo(temp.path());
        std::fs::write(temp.path().join("README.md"), "changed\n").unwrap();

        let report = run_line_ending_doctor(&GitRepo::new(temp.path()), "darwin-arm64").unwrap();

        assert!(
            report.semantic_hash_mismatches.iter().any(|mismatch| {
                mismatch.path == "README.md" && mismatch.semantic_oid.is_some()
            })
        );
        assert_has_warning(
            &report.warnings,
            LineEndingWarningCode::SemanticHashMismatch,
        );
    }

    #[test]
    fn semantic_hash_ignores_clean_filter_line_ending_differences() {
        let temp = tempfile::tempdir().unwrap();
        init_git_repo(temp.path());
        std::fs::write(temp.path().join(".gitattributes"), "* text=auto\n").unwrap();
        git(temp.path(), &["add", ".gitattributes"]);
        git(temp.path(), &["commit", "-m", "line ending policy"]);
        std::fs::write(temp.path().join("README.md"), "demo\r\n").unwrap();

        let report = run_line_ending_doctor(&GitRepo::new(temp.path()), "darwin-arm64").unwrap();

        assert!(report.semantic_hash_mismatches.is_empty());
    }

    fn assert_has_warning(warnings: &[LineEndingWarning], code: LineEndingWarningCode) {
        assert!(
            warnings.iter().any(|warning| warning.code == code),
            "{code:?}"
        );
    }

    fn init_git_repo(root: &Path) {
        git(root, &["init", "-b", "main"]);
        std::fs::write(root.join("README.md"), "demo\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "base"]);
    }

    fn git(root: &Path, args: &[&str]) {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
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
