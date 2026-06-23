//! Path portability diagnostics for cross-platform handoff safety.

use crate::{
    DevRelayError, GitRepo, Manifest, PathDecision, PlatformCapabilities, Result,
    classify_untracked_paths, platform_capabilities_for_key,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use unicode_normalization::UnicodeNormalization;

const WINDOWS_PATH_BUDGET: usize = 240;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathPortabilityDoctorReport {
    pub repo: PathBuf,
    pub target_platform_key: String,
    pub target_capabilities: PlatformCapabilities,
    pub tracked_count: usize,
    pub accepted_untracked_count: usize,
    pub issues: Vec<PathPortabilityIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathPortabilityIssue {
    pub code: PathPortabilityIssueCode,
    pub path: String,
    pub source: PathPortabilityPathSource,
    pub conflicting_paths: Vec<String>,
    pub message: String,
    pub safe_actions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum PathPortabilityIssueCode {
    CaseFoldCollision,
    UnicodeNormalizationCollision,
    WindowsReservedName,
    WindowsTrailingDotOrSpace,
    WindowsInvalidCharacter,
    PathLengthBudget,
    SymlinkUnsupportedOnTarget,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PathPortabilityPathSource {
    Tracked,
    AcceptedUntracked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PathEntry {
    path: String,
    source: PathPortabilityPathSource,
    symlink: bool,
}

pub fn run_path_portability_doctor(
    repo: &GitRepo,
    manifest: &Manifest,
    target_platform_key: &str,
) -> Result<PathPortabilityDoctorReport> {
    repo.run(&["rev-parse", "--git-dir"])?;
    let mut entries = tracked_path_entries(repo)?;
    let tracked_count = entries.len();
    let accepted_untracked = accepted_untracked_path_entries(repo, manifest)?;
    let accepted_untracked_count = accepted_untracked.len();
    entries.extend(accepted_untracked);
    let target_capabilities = platform_capabilities_for_key(target_platform_key);
    let issues = analyze_path_entries(&entries, &target_capabilities);

    Ok(PathPortabilityDoctorReport {
        repo: repo.path().to_path_buf(),
        target_platform_key: target_platform_key.to_string(),
        target_capabilities,
        tracked_count,
        accepted_untracked_count,
        issues,
    })
}

fn tracked_path_entries(repo: &GitRepo) -> Result<Vec<PathEntry>> {
    let raw = repo.run(&["ls-files", "-s", "-z"])?;
    let mut entries = Vec::new();
    for record in raw.split('\0').filter(|record| !record.is_empty()) {
        let Some((metadata, path)) = record.split_once('\t') else {
            return Err(DevRelayError::Config(format!(
                "unexpected git ls-files record: {record:?}"
            )));
        };
        let mode = metadata.split_whitespace().next().unwrap_or_default();
        entries.push(PathEntry {
            path: normalize_repo_path(path),
            source: PathPortabilityPathSource::Tracked,
            symlink: mode == "120000",
        });
    }
    Ok(entries)
}

fn accepted_untracked_path_entries(repo: &GitRepo, manifest: &Manifest) -> Result<Vec<PathEntry>> {
    let status = repo.status()?;
    let classified = classify_untracked_paths(repo.path(), manifest, status.untracked_paths())?;
    Ok(classified
        .into_iter()
        .filter(|path| path.decision == PathDecision::Include)
        .map(|path| {
            let symlink = repo
                .path()
                .join(PathBuf::from(&path.path))
                .symlink_metadata()
                .map(|metadata| metadata.file_type().is_symlink())
                .unwrap_or(false);
            PathEntry {
                path: normalize_repo_path(&path.path),
                source: PathPortabilityPathSource::AcceptedUntracked,
                symlink,
            }
        })
        .collect())
}

fn analyze_path_entries(
    entries: &[PathEntry],
    target_capabilities: &PlatformCapabilities,
) -> Vec<PathPortabilityIssue> {
    let mut issues = Vec::new();
    issues.extend(collision_issues(
        entries,
        PathPortabilityIssueCode::CaseFoldCollision,
        |path| path.to_lowercase(),
        "Paths differ only by case and can collide on case-insensitive targets.",
    ));
    issues.extend(collision_issues(
        entries,
        PathPortabilityIssueCode::UnicodeNormalizationCollision,
        |path| path.nfc().collect::<String>(),
        "Paths normalize to the same Unicode form and can collide across filesystems.",
    ));

    for entry in entries {
        detect_component_issues(entry, &mut issues);
        if entry.path.chars().count() > WINDOWS_PATH_BUDGET {
            issues.push(issue(
                PathPortabilityIssueCode::PathLengthBudget,
                entry,
                Vec::new(),
                format!(
                    "Path exceeds the portable Windows path budget of {WINDOWS_PATH_BUDGET} characters."
                ),
            ));
        }
        if entry.symlink && !target_capabilities.symlinks {
            issues.push(issue(
                PathPortabilityIssueCode::SymlinkUnsupportedOnTarget,
                entry,
                Vec::new(),
                "Path is a symlink but the target platform does not support symlink materialization."
                    .to_string(),
            ));
        }
    }

    dedupe_issues(issues)
}

fn collision_issues(
    entries: &[PathEntry],
    code: PathPortabilityIssueCode,
    key_for: impl Fn(&str) -> String,
    message: &str,
) -> Vec<PathPortabilityIssue> {
    let mut groups: BTreeMap<String, Vec<&PathEntry>> = BTreeMap::new();
    for entry in entries {
        groups.entry(key_for(&entry.path)).or_default().push(entry);
    }
    groups
        .into_values()
        .filter_map(|group| {
            let unique = group
                .iter()
                .map(|entry| entry.path.clone())
                .collect::<BTreeSet<_>>();
            if unique.len() < 2 {
                return None;
            }
            let first = group[0];
            Some(issue(
                code,
                first,
                unique.into_iter().collect(),
                message.to_string(),
            ))
        })
        .collect()
}

fn detect_component_issues(entry: &PathEntry, issues: &mut Vec<PathPortabilityIssue>) {
    for component in entry.path.split('/') {
        if component.is_empty() {
            continue;
        }
        if is_windows_reserved_name(component) {
            issues.push(issue(
                PathPortabilityIssueCode::WindowsReservedName,
                entry,
                vec![component.to_string()],
                format!("Path component {component:?} is a reserved Windows device name."),
            ));
        }
        if component.ends_with('.') || component.ends_with(' ') {
            issues.push(issue(
                PathPortabilityIssueCode::WindowsTrailingDotOrSpace,
                entry,
                vec![component.to_string()],
                format!("Path component {component:?} ends with a dot or space."),
            ));
        }
        if component.chars().any(is_invalid_windows_path_char) {
            issues.push(issue(
                PathPortabilityIssueCode::WindowsInvalidCharacter,
                entry,
                vec![component.to_string()],
                format!("Path component {component:?} contains a Windows-invalid character."),
            ));
        }
    }
}

fn issue(
    code: PathPortabilityIssueCode,
    entry: &PathEntry,
    conflicting_paths: Vec<String>,
    message: String,
) -> PathPortabilityIssue {
    PathPortabilityIssue {
        code,
        path: entry.path.clone(),
        source: entry.source,
        conflicting_paths,
        message,
        safe_actions: safe_actions_for(code),
    }
}

fn safe_actions_for(code: PathPortabilityIssueCode) -> Vec<String> {
    match code {
        PathPortabilityIssueCode::SymlinkUnsupportedOnTarget => vec![
            "Replace the symlink with a regular file or target-supported equivalent before handoff."
                .to_string(),
            "Add this path or a parent pattern to [workspace.exclude] for unsupported targets."
                .to_string(),
        ],
        _ => vec![
            "Rename the path in Git to avoid target filesystem ambiguity.".to_string(),
            "Add this path or a parent pattern to [workspace.exclude] if it should not transfer."
                .to_string(),
        ],
    }
}

fn dedupe_issues(issues: Vec<PathPortabilityIssue>) -> Vec<PathPortabilityIssue> {
    let mut seen = BTreeSet::new();
    issues
        .into_iter()
        .filter(|issue| {
            seen.insert((
                issue.code,
                issue.path.clone(),
                issue.conflicting_paths.clone(),
            ))
        })
        .collect()
}

fn normalize_repo_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn is_windows_reserved_name(component: &str) -> bool {
    let trimmed = component.trim_end_matches(['.', ' ']);
    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "CONIN$"
            | "CONOUT$"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn is_invalid_windows_path_char(ch: char) -> bool {
    matches!(ch, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*') || ch.is_control()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Manifest;
    use std::path::Path;
    use std::process::Command;

    #[test]
    fn detects_case_unicode_windows_and_length_path_issues() {
        let entries = vec![
            tracked("Readme.md", false),
            tracked("README.md", false),
            tracked("caf\u{e9}.txt", false),
            tracked("cafe\u{301}.txt", false),
            tracked("docs/CON.txt", false),
            tracked("bad/name?.txt", false),
            tracked("trailing/name. ", false),
            tracked(&format!("deep/{}.txt", "a".repeat(250)), false),
        ];

        let issues = analyze_path_entries(
            &entries,
            &platform_capabilities_for_key("windows-native-x86_64"),
        );

        assert_has_code(&issues, PathPortabilityIssueCode::CaseFoldCollision);
        assert_has_code(
            &issues,
            PathPortabilityIssueCode::UnicodeNormalizationCollision,
        );
        assert_has_code(&issues, PathPortabilityIssueCode::WindowsReservedName);
        assert_has_code(&issues, PathPortabilityIssueCode::WindowsInvalidCharacter);
        assert_has_code(&issues, PathPortabilityIssueCode::WindowsTrailingDotOrSpace);
        assert_has_code(&issues, PathPortabilityIssueCode::PathLengthBudget);
        assert!(issues.iter().all(|issue| !issue.safe_actions.is_empty()));
    }

    #[test]
    fn detects_symlink_capability_mismatch() {
        let issues = analyze_path_entries(
            &[tracked("linked-config", true)],
            &platform_capabilities_for_key("windows-native-x86_64"),
        );

        assert_has_code(
            &issues,
            PathPortabilityIssueCode::SymlinkUnsupportedOnTarget,
        );
    }

    #[test]
    fn path_doctor_walks_tracked_and_accepted_untracked_paths() {
        let temp = tempfile::tempdir().unwrap();
        init_git_repo(temp.path());
        std::fs::write(temp.path().join("CON.txt"), "reserved\n").unwrap();
        git(temp.path(), &["add", "CON.txt"]);
        git(temp.path(), &["commit", "-m", "reserved"]);
        std::fs::write(temp.path().join("scratch?.txt"), "untracked\n").unwrap();
        let manifest = Manifest::parse(
            r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#,
        )
        .unwrap();

        let report = run_path_portability_doctor(
            &GitRepo::new(temp.path()),
            &manifest,
            "windows-native-x86_64",
        )
        .unwrap();

        assert!(report.tracked_count >= 2);
        assert_eq!(report.accepted_untracked_count, 1);
        assert!(report.issues.iter().any(|issue| {
            issue.code == PathPortabilityIssueCode::WindowsReservedName
                && issue.path == "CON.txt"
                && issue.source == PathPortabilityPathSource::Tracked
        }));
        assert!(report.issues.iter().any(|issue| {
            issue.code == PathPortabilityIssueCode::WindowsInvalidCharacter
                && issue.path == "scratch?.txt"
                && issue.source == PathPortabilityPathSource::AcceptedUntracked
        }));
    }

    fn tracked(path: &str, symlink: bool) -> PathEntry {
        PathEntry {
            path: path.to_string(),
            source: PathPortabilityPathSource::Tracked,
            symlink,
        }
    }

    fn assert_has_code(issues: &[PathPortabilityIssue], code: PathPortabilityIssueCode) {
        assert!(issues.iter().any(|issue| issue.code == code), "{code:?}");
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
