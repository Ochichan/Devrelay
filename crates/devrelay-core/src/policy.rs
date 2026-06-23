//! Untracked-file and secret-safety classification.
//!
//! Snapshot creation asks this module which untracked paths can be carried
//! forward. Secret-like names and private key material are hard-blocked before
//! manifest include rules are considered. Generated directories and large files
//! are excluded to keep local handoffs intentional and reviewable.

use crate::error::Result;
use crate::fs_safety::{is_traversal_boundary, is_windows_reparse_point};
use crate::manifest::{Manifest, SecretScannerConfig, UntrackedPolicy};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

pub mod classification_reason {
    pub const SECRET_FILENAME: &str = "secret-filename";
    pub const SSH_CREDENTIAL_PATH: &str = "ssh-credential-path";
    pub const PRIVATE_KEY_FILENAME: &str = "private-key-filename";
    pub const PRIVATE_KEY_CONTENT: &str = "private-key-content";
    pub const TOKEN_PATTERN_CONTENT: &str = "token-pattern-content";
    pub const HIGH_ENTROPY_CONTENT: &str = "high-entropy-content";
    pub const HIGH_ENTROPY_PLACEHOLDER: &str = HIGH_ENTROPY_CONTENT;
    pub const USER_SECRET_SCANNER: &str = "user-secret-scanner";
    pub const WINDOWS_REPARSE_POINT: &str = "windows-reparse-point";
    pub const SYMLINK_TARGET_OUTSIDE_WORKSPACE: &str = "symlink-target-outside-workspace";
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
    let user_secret_paths = build_globset(
        manifest
            .workspace
            .secret_scanner
            .filename_patterns
            .iter()
            .map(String::as_str),
    )?;
    let threshold_bytes = manifest.workspace.large_file_threshold_mib * 1024 * 1024;
    let context = ClassificationContext {
        repo_root,
        policy: manifest.workspace.untracked,
        exclude: &exclude,
        include: &include,
        scanner: &manifest.workspace.secret_scanner,
        user_secret_paths: &user_secret_paths,
        threshold_bytes,
    };
    let mut classified = Vec::new();

    for path_ref in paths {
        let path = normalize_repo_path(path_ref.as_ref());
        let decision = classify_one(&context, &path);
        classified.push(decision);
    }

    Ok(classified)
}

struct ClassificationContext<'a> {
    repo_root: &'a Path,
    policy: UntrackedPolicy,
    exclude: &'a GlobSet,
    include: &'a GlobSet,
    scanner: &'a SecretScannerConfig,
    user_secret_paths: &'a GlobSet,
    threshold_bytes: u64,
}

fn classify_one(context: &ClassificationContext<'_>, path: &str) -> ClassifiedPath {
    if let Some(reason) = secret_path_reason(path) {
        return excluded(path, reason);
    }
    if context.user_secret_paths.is_match(path) {
        return excluded(path, classification_reason::USER_SECRET_SCANNER);
    }
    if path_is_unsupported_windows_reparse_point(context.repo_root, path) {
        return excluded(path, classification_reason::WINDOWS_REPARSE_POINT);
    }
    if symlink_target_escapes_workspace(context.repo_root, path) {
        return excluded(
            path,
            classification_reason::SYMLINK_TARGET_OUTSIDE_WORKSPACE,
        );
    }
    if let Some(reason) = secret_content_reason(context.repo_root, path, context.scanner) {
        return excluded(path, reason);
    }
    if context.exclude.is_match(path) {
        return excluded(path, classification_reason::MANIFEST_OR_GENERATED_EXCLUDE);
    }
    if exceeds_threshold(context.repo_root, path, context.threshold_bytes) {
        return excluded(path, classification_reason::LARGE_FILE_THRESHOLD);
    }

    match context.policy {
        UntrackedPolicy::None => excluded(path, classification_reason::MANIFEST_UNTRACKED_NONE),
        UntrackedPolicy::Safe | UntrackedPolicy::AllNonignored => {
            included(path, classification_reason::SAFE_UNTRACKED)
        }
        UntrackedPolicy::Explicit => {
            if context.include.is_match(path) {
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
    if basename == ".env"
        || basename.starts_with(".env.")
        || matches!(
            basename,
            ".npmrc"
                | ".pypirc"
                | ".netrc"
                | "auth.json"
                | "credentials"
                | "credentials.json"
                | "secret.json"
                | "secrets.json"
                | "service-account.json"
        )
        || basename.ends_with(".secret")
        || basename.ends_with(".secrets")
        || basename.ends_with(".credentials")
        || basename.ends_with(".p12")
        || basename.ends_with(".pfx")
        || basename.ends_with(".jks")
        || basename.ends_with(".keystore")
        || basename.ends_with(".kdbx")
        || basename.ends_with(".age")
        || basename.ends_with(".gpg")
        || basename.contains("firebase-adminsdk")
        || basename.contains("service_account")
        || basename.contains("service-account")
        || basename.contains("client-secret")
        || basename.contains("client_secret")
    {
        return Some(classification_reason::SECRET_FILENAME);
    }
    if normalized == ".ssh" || normalized.starts_with(".ssh/") || normalized.contains("/.ssh/") {
        return Some(classification_reason::SSH_CREDENTIAL_PATH);
    }
    if basename.ends_with(".pem")
        || basename.ends_with(".key")
        || basename == "id_rsa"
        || basename == "id_ed25519"
        || basename == "id_ecdsa"
        || basename == "id_dsa"
        || basename == "id_xmss"
        || basename == "id_ed25519_sk"
        || basename == "id_ecdsa_sk"
    {
        return Some(classification_reason::PRIVATE_KEY_FILENAME);
    }
    None
}

fn secret_content_reason(
    repo_root: &Path,
    path: &str,
    scanner: &SecretScannerConfig,
) -> Option<&'static str> {
    let full = repo_root.join(PathBuf::from(path));
    let Ok(metadata) = fs::symlink_metadata(&full) else {
        return None;
    };
    if is_traversal_boundary(&metadata) {
        return None;
    }
    let Ok(mut file) = File::open(full) else {
        return None;
    };
    let mut buffer = [0_u8; 64 * 1024];
    let Ok(read) = file.read(&mut buffer) else {
        return None;
    };
    let haystack = String::from_utf8_lossy(&buffer[..read]);
    if has_private_key_header(&haystack) {
        return Some(classification_reason::PRIVATE_KEY_CONTENT);
    }
    if has_user_secret_marker(&haystack, scanner) || has_user_token_prefix(&haystack, scanner) {
        return Some(classification_reason::USER_SECRET_SCANNER);
    }
    if has_token_pattern(&haystack) {
        return Some(classification_reason::TOKEN_PATTERN_CONTENT);
    }
    if has_high_entropy_secret(&haystack) {
        return Some(classification_reason::HIGH_ENTROPY_CONTENT);
    }
    None
}

fn has_user_secret_marker(haystack: &str, scanner: &SecretScannerConfig) -> bool {
    scanner
        .content_markers
        .iter()
        .any(|marker| !marker.is_empty() && haystack.contains(marker))
}

fn has_user_token_prefix(haystack: &str, scanner: &SecretScannerConfig) -> bool {
    haystack.lines().any(|line| {
        token_candidates(line).any(|token| {
            let token = trim_token_value(token);
            scanner.token_prefixes.iter().any(|prefix| {
                !prefix.is_empty() && token.starts_with(prefix) && token.len() >= prefix.len() + 8
            })
        })
    })
}

fn has_private_key_header(haystack: &str) -> bool {
    haystack.contains("BEGIN PRIVATE KEY")
        || haystack.contains("BEGIN RSA PRIVATE KEY")
        || haystack.contains("BEGIN OPENSSH PRIVATE KEY")
        || haystack.contains("BEGIN EC PRIVATE KEY")
        || haystack.contains("BEGIN DSA PRIVATE KEY")
        || haystack.contains("BEGIN ENCRYPTED PRIVATE KEY")
        || haystack.contains("BEGIN PGP PRIVATE KEY BLOCK")
}

fn has_token_pattern(haystack: &str) -> bool {
    haystack.lines().any(|line| {
        token_candidates(line).any(looks_like_known_token)
            || assignment_value(line).is_some_and(|value| {
                let value = trim_token_value(value);
                looks_like_known_token(value)
            })
    })
}

fn has_high_entropy_secret(haystack: &str) -> bool {
    haystack.lines().any(|line| {
        if !has_secret_context(line) {
            return false;
        }
        token_candidates(line).any(|candidate| {
            let candidate = trim_token_value(candidate);
            candidate.len() >= 32
                && is_secret_token_charset(candidate)
                && token_entropy(candidate) >= 4.2
        })
    })
}

fn has_secret_context(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "api_key",
        "apikey",
        "access_key",
        "access-token",
        "access_token",
        "auth_token",
        "client_secret",
        "client-secret",
        "password",
        "private_key",
        "private-key",
        "secret",
        "token",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn assignment_value(line: &str) -> Option<&str> {
    if !has_secret_context(line) {
        return None;
    }
    line.split_once('=')
        .or_else(|| line.split_once(':'))
        .map(|(_, value)| value.trim())
}

fn token_candidates(line: &str) -> impl Iterator<Item = &str> {
    line.split(|ch: char| {
        !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '+' | '='))
    })
    .filter(|candidate| !candidate.is_empty())
}

fn trim_token_value(value: &str) -> &str {
    value.trim_matches(|ch: char| {
        ch == '"'
            || ch == '\''
            || ch == '`'
            || ch == ','
            || ch == ';'
            || ch == ')'
            || ch == ']'
            || ch == '}'
    })
}

fn looks_like_known_token(token: &str) -> bool {
    let token = trim_token_value(token);
    if token.len() == 20
        && (token.starts_with("AKIA") || token.starts_with("ASIA"))
        && token
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return true;
    }
    if token.starts_with("github_pat_") && token.len() >= 40 {
        return true;
    }
    if ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
        .iter()
        .any(|prefix| token.starts_with(prefix))
        && token.len() >= 30
    {
        return true;
    }
    if ["xoxb-", "xoxa-", "xoxp-", "xoxr-", "xoxs-"]
        .iter()
        .any(|prefix| token.starts_with(prefix))
        && token.len() >= 20
    {
        return true;
    }
    if token.starts_with("sk-") && token.len() >= 32 {
        return token_entropy(token) >= 4.0;
    }
    false
}

fn is_secret_token_charset(value: &str) -> bool {
    value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b'+' | b'=')
    })
}

fn token_entropy(value: &str) -> f64 {
    if value.is_empty() {
        return 0.0;
    }
    let mut counts = [0_usize; 256];
    for byte in value.bytes() {
        counts[byte as usize] += 1;
    }
    let len = value.len() as f64;
    counts
        .into_iter()
        .filter(|count| *count > 0)
        .map(|count| {
            let probability = count as f64 / len;
            -probability * probability.log2()
        })
        .sum()
}

fn exceeds_threshold(repo_root: &Path, path: &str, threshold_bytes: u64) -> bool {
    if threshold_bytes == 0 {
        return false;
    }
    let full = repo_root.join(PathBuf::from(path));
    let Ok(metadata) = fs::symlink_metadata(full) else {
        return false;
    };
    if is_traversal_boundary(&metadata) {
        return false;
    }
    metadata.is_file() && metadata.len() > threshold_bytes
}

fn path_is_unsupported_windows_reparse_point(repo_root: &Path, path: &str) -> bool {
    let full = repo_root.join(PathBuf::from(path));
    let Ok(metadata) = fs::symlink_metadata(&full) else {
        return false;
    };
    is_windows_reparse_point(&metadata) && !metadata.file_type().is_symlink()
}

fn symlink_target_escapes_workspace(repo_root: &Path, path: &str) -> bool {
    let full = repo_root.join(PathBuf::from(path));
    let Ok(metadata) = fs::symlink_metadata(&full) else {
        return false;
    };
    if !metadata.file_type().is_symlink() {
        return false;
    }
    let Ok(target) = fs::read_link(&full) else {
        return true;
    };
    if target.is_absolute() {
        return true;
    }
    let link_parent = Path::new(path).parent().unwrap_or_else(|| Path::new(""));
    normalize_workspace_relative_path(&link_parent.join(target)).is_none()
}

pub fn normalize_workspace_relative_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            std::path::Component::Prefix(_) | std::path::Component::RootDir => return None,
        }
    }
    Some(normalized)
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
        let decisions = decisions(&[
            ".env",
            ".env.local",
            "config/.env.production",
            ".npmrc",
            ".pypirc",
            ".netrc",
            "credentials.json",
            "service-account.json",
            "firebase-adminsdk-prod.json",
            "prod.client-secret",
            "vault.kdbx",
            "notes.md",
        ]);
        assert_eq!(decisions[0].decision, PathDecision::Exclude);
        assert_eq!(decisions[0].reason, classification_reason::SECRET_FILENAME);
        for decision in &decisions[1..11] {
            assert_eq!(decision.decision, PathDecision::Exclude);
            assert_eq!(decision.reason, classification_reason::SECRET_FILENAME);
        }
        assert_eq!(decisions[11].decision, PathDecision::Include);
    }

    #[test]
    fn excludes_ssh_paths_and_private_key_filenames() {
        let decisions = decisions(&[
            ".ssh/id_rsa",
            "nested/.ssh/config",
            "deploy.pem",
            "deploy.key",
            "id_ed25519",
            "id_ecdsa",
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
        assert_eq!(decisions[5].decision, PathDecision::Exclude);
        assert_eq!(
            decisions[5].reason,
            classification_reason::PRIVATE_KEY_FILENAME
        );
        assert_eq!(decisions[6].decision, PathDecision::Include);
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
    fn excludes_token_pattern_content() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("notes.txt"),
            "temporary token ghp_abcdefghijklmnopqrstuvwxyz1234567890\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("aws.txt"),
            "aws_access_key_id=AKIA1234567890ABCDEF\n",
        )
        .unwrap();

        let decisions =
            classify_untracked_paths(temp.path(), &safe_manifest(), ["notes.txt", "aws.txt"])
                .unwrap();

        for decision in decisions {
            assert_eq!(decision.decision, PathDecision::Exclude);
            assert_eq!(
                decision.reason,
                classification_reason::TOKEN_PATTERN_CONTENT
            );
        }
    }

    #[test]
    fn excludes_high_entropy_secret_assignments() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("config.txt"),
            "api_key = \"S3cr3tAbCdEfGhIjKlMnOpQrStUvWxYz1234567890+/=\"\n",
        )
        .unwrap();

        let decisions =
            classify_untracked_paths(temp.path(), &safe_manifest(), ["config.txt"]).unwrap();

        assert_eq!(decisions[0].decision, PathDecision::Exclude);
        assert_eq!(
            decisions[0].reason,
            classification_reason::HIGH_ENTROPY_CONTENT
        );
    }

    #[test]
    fn does_not_exclude_low_entropy_secret_assignments() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("config.txt"), "token = \"development\"\n").unwrap();

        let decisions =
            classify_untracked_paths(temp.path(), &safe_manifest(), ["config.txt"]).unwrap();

        assert_eq!(decisions[0].decision, PathDecision::Include);
    }

    #[test]
    fn applies_user_configured_secret_scanner_hook() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("artifact.local-secret"),
            "safe name block\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("custom-marker.txt"),
            "BEGIN CUSTOM SECRET\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("custom-token.txt"),
            "customtok_1234567890abcdef\n",
        )
        .unwrap();
        fs::write(temp.path().join("notes.md"), "ordinary notes\n").unwrap();
        let manifest = Manifest::parse(
            r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"

[workspace.secret_scanner]
filename_patterns = ["*.local-secret"]
content_markers = ["BEGIN CUSTOM SECRET"]
token_prefixes = ["customtok_"]
"#,
        )
        .unwrap();

        let decisions = classify_untracked_paths(
            temp.path(),
            &manifest,
            [
                "artifact.local-secret",
                "custom-marker.txt",
                "custom-token.txt",
                "notes.md",
            ],
        )
        .unwrap();

        for decision in &decisions[..3] {
            assert_eq!(decision.decision, PathDecision::Exclude);
            assert_eq!(decision.reason, classification_reason::USER_SECRET_SCANNER);
        }
        assert_eq!(decisions[3].decision, PathDecision::Include);
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

    #[cfg(unix)]
    #[test]
    fn excludes_symlink_targets_outside_workspace() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("inside.txt"), "inside\n").unwrap();
        fs::write(
            temp.path().parent().unwrap().join("outside.txt"),
            "outside\n",
        )
        .unwrap();
        std::os::unix::fs::symlink("../outside.txt", temp.path().join("escape-link")).unwrap();

        let decisions =
            classify_untracked_paths(temp.path(), &safe_manifest(), ["inside.txt", "escape-link"])
                .unwrap();

        assert_eq!(decisions[0].decision, PathDecision::Include);
        assert_eq!(decisions[1].decision, PathDecision::Exclude);
        assert_eq!(
            decisions[1].reason,
            classification_reason::SYMLINK_TARGET_OUTSIDE_WORKSPACE
        );
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlink_targets_for_content_checks() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("secret.txt"),
            "-----BEGIN PRIVATE KEY-----\nsecret\n",
        )
        .unwrap();
        std::os::unix::fs::symlink("secret.txt", temp.path().join("link.txt")).unwrap();

        let decisions =
            classify_untracked_paths(temp.path(), &safe_manifest(), ["link.txt"]).unwrap();

        assert_eq!(decisions[0].decision, PathDecision::Include);
    }

    #[test]
    fn normalizes_workspace_relative_paths_without_escape() {
        assert_eq!(
            normalize_workspace_relative_path(Path::new("src/./bin/../lib.rs")),
            Some(PathBuf::from("src/lib.rs"))
        );
        assert_eq!(
            normalize_workspace_relative_path(Path::new("")),
            Some(PathBuf::new())
        );
        assert_eq!(
            normalize_workspace_relative_path(Path::new("../secret.txt")),
            None
        );
        assert_eq!(
            normalize_workspace_relative_path(Path::new("src/../../secret.txt")),
            None
        );
        assert_eq!(
            normalize_workspace_relative_path(Path::new("/tmp/secret.txt")),
            None
        );
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
