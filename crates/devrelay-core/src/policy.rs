//! Untracked-file and secret-safety classification.
//!
//! Snapshot creation asks this module which untracked paths can be carried
//! forward. Secret-like names and private key material are hard-blocked before
//! manifest include rules are considered. Generated directories and large files
//! are excluded to keep local handoffs intentional and reviewable.

use crate::error::Result;
use crate::manifest::{Manifest, UntrackedPolicy};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

pub mod classification_reason {
    pub const SECRET_FILENAME: &str = "secret-filename";
    pub const SSH_CREDENTIAL_PATH: &str = "ssh-credential-path";
    pub const PRIVATE_KEY_FILENAME: &str = "private-key-filename";
    pub const PRIVATE_KEY_CONTENT: &str = "private-key-content";
    pub const HIGH_ENTROPY_PLACEHOLDER: &str = "high-entropy-placeholder";
    pub const MANIFEST_OR_GENERATED_EXCLUDE: &str = "manifest-or-generated-exclude";
    pub const LARGE_FILE_THRESHOLD: &str = "large-file-threshold";
    pub const MANIFEST_UNTRACKED_NONE: &str = "manifest-untracked-none";
    pub const SAFE_UNTRACKED: &str = "safe-untracked";
    pub const MANIFEST_INCLUDE: &str = "manifest-include";
    pub const MANIFEST_UNTRACKED_EXPLICIT: &str = "manifest-untracked-explicit";
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClassifiedPath {
    pub path: String,
    pub decision: PathDecision,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PathDecision {
    Include,
    Exclude,
}

pub fn classify_untracked_paths(
    repo_root: &Path,
    manifest: &Manifest,
    paths: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<Vec<ClassifiedPath>> {
    let mut exclude_patterns = manifest.workspace.exclude.patterns.clone();
    exclude_patterns.extend(default_exclude_patterns().map(str::to_string));
    let exclude = build_globset(exclude_patterns.iter().map(String::as_str))?;
    let include = build_globset(
        manifest
            .workspace
            .include
            .patterns
            .iter()
            .map(String::as_str),
    )?;
    let threshold_bytes = manifest.workspace.large_file_threshold_mib * 1024 * 1024;
    let mut classified = Vec::new();

    for path_ref in paths {
        let path = normalize_repo_path(path_ref.as_ref());
        let decision = classify_one(
            repo_root,
            &path,
            manifest.workspace.untracked,
            &exclude,
            &include,
            threshold_bytes,
        );
        classified.push(decision);
    }

    Ok(classified)
}

fn classify_one(
    repo_root: &Path,
    path: &str,
    policy: UntrackedPolicy,
    exclude: &GlobSet,
    include: &GlobSet,
    threshold_bytes: u64,
) -> ClassifiedPath {
    if let Some(reason) = secret_path_reason(path) {
        return excluded(path, reason);
    }
    if path_has_private_key_header(repo_root, path) {
        return excluded(path, classification_reason::PRIVATE_KEY_CONTENT);
    }
    if has_high_entropy_secret(repo_root, path) {
        return excluded(path, classification_reason::HIGH_ENTROPY_PLACEHOLDER);
    }
    if exclude.is_match(path) {
        return excluded(path, classification_reason::MANIFEST_OR_GENERATED_EXCLUDE);
    }
    if exceeds_threshold(repo_root, path, threshold_bytes) {
        return excluded(path, classification_reason::LARGE_FILE_THRESHOLD);
    }

    match policy {
        UntrackedPolicy::None => excluded(path, classification_reason::MANIFEST_UNTRACKED_NONE),
        UntrackedPolicy::Safe | UntrackedPolicy::AllNonignored => {
            included(path, classification_reason::SAFE_UNTRACKED)
        }
        UntrackedPolicy::Explicit => {
            if include.is_match(path) {
                included(path, classification_reason::MANIFEST_INCLUDE)
            } else {
                excluded(path, classification_reason::MANIFEST_UNTRACKED_EXPLICIT)
            }
        }
    }
}

fn build_globset(patterns: impl IntoIterator<Item = impl AsRef<str>>) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern.as_ref())?);
    }
    Ok(builder.build()?)
}

fn default_exclude_patterns() -> impl Iterator<Item = &'static str> {
    [
        "node_modules/**",
        ".venv/**",
        "target/**",
        "dist/**",
        ".next/**",
        "*.sqlite-wal",
        "*.pid",
        "*.sock",
        "*.lock",
    ]
    .into_iter()
}

fn normalize_repo_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn included(path: &str, reason: &str) -> ClassifiedPath {
    ClassifiedPath {
        path: path.to_string(),
        decision: PathDecision::Include,
        reason: reason.to_string(),
    }
}

fn excluded(path: &str, reason: &str) -> ClassifiedPath {
    ClassifiedPath {
        path: path.to_string(),
        decision: PathDecision::Exclude,
        reason: reason.to_string(),
    }
}

fn secret_path_reason(path: &str) -> Option<&'static str> {
    let normalized = normalize_repo_path(path).to_ascii_lowercase();
    let basename = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    if basename == ".env" || basename.starts_with(".env.") {
        return Some(classification_reason::SECRET_FILENAME);
    }
    if normalized == ".ssh" || normalized.starts_with(".ssh/") || normalized.contains("/.ssh/") {
        return Some(classification_reason::SSH_CREDENTIAL_PATH);
    }
    if basename.ends_with(".pem")
        || basename.ends_with(".key")
        || basename == "id_rsa"
        || basename == "id_ed25519"
    {
        return Some(classification_reason::PRIVATE_KEY_FILENAME);
    }
    None
}

fn path_has_private_key_header(repo_root: &Path, path: &str) -> bool {
    let full = repo_root.join(PathBuf::from(path));
    let Ok(mut file) = File::open(full) else {
        return false;
    };
    let mut buffer = [0_u8; 8192];
    let Ok(read) = file.read(&mut buffer) else {
        return false;
    };
    let haystack = String::from_utf8_lossy(&buffer[..read]);
    haystack.contains("BEGIN PRIVATE KEY")
        || haystack.contains("BEGIN RSA PRIVATE KEY")
        || haystack.contains("BEGIN OPENSSH PRIVATE KEY")
        || haystack.contains("BEGIN EC PRIVATE KEY")
}

#[cfg(feature = "entropy-detection")]
fn has_high_entropy_secret(_repo_root: &Path, _path: &str) -> bool {
    false
}

#[cfg(not(feature = "entropy-detection"))]
fn has_high_entropy_secret(_repo_root: &Path, _path: &str) -> bool {
    false
}

fn exceeds_threshold(repo_root: &Path, path: &str, threshold_bytes: u64) -> bool {
    if threshold_bytes == 0 {
        return false;
    }
    let full = repo_root.join(PathBuf::from(path));
    let Ok(metadata) = full.metadata() else {
        return false;
    };
    metadata.is_file() && metadata.len() > threshold_bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Manifest;
    use std::fs;

    fn manifest(untracked: &str, include_patterns: &[&str], threshold_mib: u64) -> Manifest {
        let include = if include_patterns.is_empty() {
            String::new()
        } else {
            format!(
                r#"
[workspace.include]
patterns = [{}]
"#,
                include_patterns
                    .iter()
                    .map(|pattern| format!("\"{pattern}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        Manifest::parse(&format!(
            r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "{untracked}"
portable_paths = "strict"
large_file_threshold_mib = {threshold_mib}
{include}
"#,
        ))
        .unwrap()
    }

    fn safe_manifest() -> Manifest {
        manifest("safe", &[], 32)
    }

    fn decisions(paths: &[&str]) -> Vec<ClassifiedPath> {
        classify_untracked_paths(Path::new("."), &safe_manifest(), paths).unwrap()
    }

    #[test]
    fn excludes_secret_names() {
        let decisions = decisions(&[".env", ".env.local", "config/.env.production", "notes.md"]);
        assert_eq!(decisions[0].decision, PathDecision::Exclude);
        assert_eq!(decisions[0].reason, classification_reason::SECRET_FILENAME);
        assert_eq!(decisions[1].decision, PathDecision::Exclude);
        assert_eq!(decisions[2].decision, PathDecision::Exclude);
        assert_eq!(decisions[3].decision, PathDecision::Include);
    }

    #[test]
    fn excludes_ssh_paths_and_private_key_filenames() {
        let decisions = decisions(&[
            ".ssh/id_rsa",
            "nested/.ssh/config",
            "deploy.pem",
            "deploy.key",
            "id_ed25519",
            "src/main.rs",
        ]);

        for decision in &decisions[..2] {
            assert_eq!(decision.decision, PathDecision::Exclude);
            assert_eq!(decision.reason, classification_reason::SSH_CREDENTIAL_PATH);
        }
        for decision in &decisions[2..5] {
            assert_eq!(decision.decision, PathDecision::Exclude);
            assert_eq!(decision.reason, classification_reason::PRIVATE_KEY_FILENAME);
        }
        assert_eq!(decisions[5].decision, PathDecision::Include);
    }

    #[test]
    fn excludes_private_key_content() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("notes.txt"),
            "-----BEGIN OPENSSH PRIVATE KEY-----\nsecret\n",
        )
        .unwrap();

        let decisions =
            classify_untracked_paths(temp.path(), &safe_manifest(), ["notes.txt"]).unwrap();
        assert_eq!(decisions[0].decision, PathDecision::Exclude);
        assert_eq!(
            decisions[0].reason,
            classification_reason::PRIVATE_KEY_CONTENT
        );
    }

    #[test]
    fn excludes_generated_and_transient_paths() {
        let decisions = decisions(&[
            "node_modules/pkg/index.js",
            ".venv/bin/python",
            "target/debug/app",
            "dist/app.js",
            ".next/cache/data",
            "server.sock",
            "app.pid",
            "db.lock",
        ]);

        assert!(decisions.iter().all(|decision| {
            decision.decision == PathDecision::Exclude
                && decision.reason == classification_reason::MANIFEST_OR_GENERATED_EXCLUDE
        }));
    }

    #[test]
    fn excludes_files_over_large_file_threshold() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("large.bin"), vec![0_u8; 1024 * 1024 + 1]).unwrap();

        let manifest = manifest("safe", &[], 1);
        let decisions =
            classify_untracked_paths(temp.path(), &manifest, ["large.bin", "missing.bin"]).unwrap();

        assert_eq!(decisions[0].decision, PathDecision::Exclude);
        assert_eq!(
            decisions[0].reason,
            classification_reason::LARGE_FILE_THRESHOLD
        );
        assert_eq!(decisions[1].decision, PathDecision::Include);
    }

    #[test]
    fn applies_untracked_none_policy() {
        let manifest = manifest("none", &[], 32);
        let decisions = classify_untracked_paths(Path::new("."), &manifest, ["notes.md"]).unwrap();

        assert_eq!(decisions[0].decision, PathDecision::Exclude);
        assert_eq!(
            decisions[0].reason,
            classification_reason::MANIFEST_UNTRACKED_NONE
        );
    }

    #[test]
    fn applies_untracked_safe_policy() {
        let decisions = decisions(&["notes.md"]);

        assert_eq!(decisions[0].decision, PathDecision::Include);
        assert_eq!(decisions[0].reason, classification_reason::SAFE_UNTRACKED);
    }

    #[test]
    fn applies_untracked_all_nonignored_policy() {
        let manifest = manifest("all-nonignored", &[], 32);
        let decisions =
            classify_untracked_paths(Path::new("."), &manifest, ["notes.md", ".env"]).unwrap();

        assert_eq!(decisions[0].decision, PathDecision::Include);
        assert_eq!(decisions[0].reason, classification_reason::SAFE_UNTRACKED);
        assert_eq!(decisions[1].decision, PathDecision::Exclude);
        assert_eq!(decisions[1].reason, classification_reason::SECRET_FILENAME);
    }

    #[test]
    fn applies_untracked_explicit_policy() {
        let manifest = manifest("explicit", &["notes/**"], 32);
        let decisions =
            classify_untracked_paths(Path::new("."), &manifest, ["notes/todo.md", "scratch.txt"])
                .unwrap();

        assert_eq!(decisions[0].decision, PathDecision::Include);
        assert_eq!(decisions[0].reason, classification_reason::MANIFEST_INCLUDE);
        assert_eq!(decisions[1].decision, PathDecision::Exclude);
        assert_eq!(
            decisions[1].reason,
            classification_reason::MANIFEST_UNTRACKED_EXPLICIT
        );
    }
}
