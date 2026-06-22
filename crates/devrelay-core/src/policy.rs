use crate::error::Result;
use crate::manifest::{Manifest, UntrackedPolicy};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

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
        return excluded(path, "private-key-content");
    }
    if exclude.is_match(path) {
        return excluded(path, "manifest-or-generated-exclude");
    }
    if exceeds_threshold(repo_root, path, threshold_bytes) {
        return excluded(path, "large-file-threshold");
    }

    match policy {
        UntrackedPolicy::None => excluded(path, "manifest-untracked-none"),
        UntrackedPolicy::Safe | UntrackedPolicy::AllNonignored => included(path, "safe-untracked"),
        UntrackedPolicy::Explicit => {
            if include.is_match(path) {
                included(path, "manifest-include")
            } else {
                excluded(path, "manifest-untracked-explicit")
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
        return Some("secret-filename");
    }
    if normalized == ".ssh" || normalized.starts_with(".ssh/") || normalized.contains("/.ssh/") {
        return Some("ssh-credential-path");
    }
    if basename.ends_with(".pem")
        || basename.ends_with(".key")
        || basename == "id_rsa"
        || basename == "id_ed25519"
    {
        return Some("private-key-filename");
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

    fn manifest() -> Manifest {
        Manifest::parse(
            r#"
schema = 1
project_id = "12345678"
name = "demo"

[workspace]
untracked = "safe"
portable_paths = "strict"
large_file_threshold_mib = 32
"#,
        )
        .unwrap()
    }

    #[test]
    fn excludes_secret_names() {
        let decisions =
            classify_untracked_paths(Path::new("."), &manifest(), [".env", "notes.md"]).unwrap();
        assert_eq!(decisions[0].decision, PathDecision::Exclude);
        assert_eq!(decisions[1].decision, PathDecision::Include);
    }
}
