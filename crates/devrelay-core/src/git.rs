//! Git CLI orchestration and porcelain status parsing.
//!
//! DevRelay uses Git's installed CLI as the M0 authority for repository state.
//! This module wraps command execution narrowly and parses
//! `git status --porcelain=v2 -z --branch` into typed status data. Unknown
//! porcelain headers are ignored so newer Git versions can add metadata without
//! breaking status collection.

use crate::error::{DevRelayError, Result};
use serde::{Deserialize, Serialize};
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[derive(Debug, Clone)]
pub struct GitRepo {
    path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitStatus {
    pub head_oid: String,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub entries: Vec<StatusEntry>,
    pub counts: StatusCounts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusEntry {
    pub kind: StatusEntryKind,
    pub xy: Option<String>,
    pub path: String,
    pub original_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StatusEntryKind {
    Ordinary,
    Renamed,
    Copied,
    Unmerged,
    Untracked,
    Ignored,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusCounts {
    pub staged: usize,
    pub unstaged: usize,
    pub untracked: usize,
    pub ignored: usize,
    pub unmerged: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusSummary {
    pub head_oid: String,
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub counts: StatusCounts,
    pub clean: bool,
    pub initial: bool,
}

impl GitStatus {
    pub fn is_clean(&self) -> bool {
        self.counts.staged == 0
            && self.counts.unstaged == 0
            && self.counts.untracked == 0
            && self.counts.unmerged == 0
    }

    pub fn is_initial(&self) -> bool {
        self.head_oid == "(initial)"
    }

    pub fn untracked_paths(&self) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter(|entry| entry.kind == StatusEntryKind::Untracked)
            .map(|entry| entry.path.as_str())
    }

    pub fn short_summary(&self) -> String {
        format!(
            "{} staged, {} unstaged, {} untracked, {} unmerged",
            self.counts.staged, self.counts.unstaged, self.counts.untracked, self.counts.unmerged
        )
    }

    pub fn summary(&self) -> StatusSummary {
        StatusSummary {
            head_oid: self.head_oid.clone(),
            branch: self.branch.clone(),
            upstream: self.upstream.clone(),
            counts: self.counts.clone(),
            clean: self.is_clean(),
            initial: self.is_initial(),
        }
    }
}

impl GitRepo {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn run(&self, args: &[&str]) -> Result<String> {
        self.run_os(args.iter().map(OsString::from), &[])
    }

    pub fn run_with_env<I>(&self, args: I, envs: &[(&str, &OsStr)]) -> Result<String>
    where
        I: IntoIterator<Item = OsString>,
    {
        self.run_os(args, envs)
    }

    pub fn run_with_stdin(&self, args: &[&str], input: &[u8]) -> Result<String> {
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.path);
        command.args(&args);
        command.stdin(Stdio::piped());
        let mut child = command.spawn()?;
        child
            .stdin
            .as_mut()
            .expect("stdin should be piped")
            .write_all(input)?;
        let output = child.wait_with_output()?;
        self.output_to_result(args, output)
    }

    fn run_os<I>(&self, args: I, envs: &[(&str, &OsStr)]) -> Result<String>
    where
        I: IntoIterator<Item = OsString>,
    {
        let args: Vec<OsString> = args.into_iter().collect();
        let mut command = Command::new("git");
        command.arg("-C").arg(&self.path);
        for (key, value) in envs.iter().copied() {
            command.env(key, value);
        }
        command.args(&args);
        let output = command.output()?;
        self.output_to_result(args, output)
    }

    fn output_to_result(&self, args: Vec<OsString>, output: Output) -> Result<String> {
        if !output.status.success() {
            return Err(DevRelayError::GitCommand {
                cwd: self.path.clone(),
                args: args
                    .iter()
                    .map(|arg| arg.to_string_lossy())
                    .collect::<Vec<_>>()
                    .join(" "),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string())
    }

    pub fn git_dir(&self) -> Result<PathBuf> {
        let raw = self.run(&["rev-parse", "--git-dir"])?;
        let path = PathBuf::from(raw);
        if path.is_absolute() {
            Ok(path)
        } else {
            Ok(self.path.join(path))
        }
    }

    pub fn status(&self) -> Result<GitStatus> {
        let raw = self.run(&[
            "status",
            "--porcelain=v2",
            "-z",
            "--branch",
            "--untracked-files=all",
        ])?;
        parse_status_porcelain_v2(&raw)
    }

    pub fn current_index_tree(&self) -> Result<String> {
        self.run(&["write-tree"])
    }
}

#[doc(hidden)]
pub fn parse_status_porcelain_v2(raw: &str) -> Result<GitStatus> {
    let records: Vec<&str> = raw
        .split('\0')
        .filter(|record| !record.is_empty())
        .collect();
    let mut head_oid = String::new();
    let mut branch = None;
    let mut upstream = None;
    let mut entries = Vec::new();
    let mut counts = StatusCounts::default();
    let mut index = 0;

    while index < records.len() {
        let record = records[index];
        if let Some(value) = record.strip_prefix("# branch.oid ") {
            head_oid = value.to_string();
            index += 1;
            continue;
        }
        if let Some(value) = record.strip_prefix("# branch.head ") {
            if value != "(detached)" {
                branch = Some(value.to_string());
            }
            index += 1;
            continue;
        }
        if let Some(value) = record.strip_prefix("# branch.upstream ") {
            upstream = Some(value.to_string());
            index += 1;
            continue;
        }

        if let Some(path) = record.strip_prefix("? ") {
            counts.untracked += 1;
            entries.push(StatusEntry {
                kind: StatusEntryKind::Untracked,
                xy: None,
                path: path.to_string(),
                original_path: None,
            });
            index += 1;
            continue;
        }

        if let Some(path) = record.strip_prefix("! ") {
            counts.ignored += 1;
            entries.push(StatusEntry {
                kind: StatusEntryKind::Ignored,
                xy: None,
                path: path.to_string(),
                original_path: None,
            });
            index += 1;
            continue;
        }

        if record.starts_with("1 ") || record.starts_with("2 ") {
            let kind = changed_entry_kind(record);
            let xy = record.split_whitespace().nth(1).ok_or_else(|| {
                DevRelayError::Manifest(format!("invalid git status record: {record}"))
            })?;
            add_xy_counts(xy, &mut counts);
            let path = parse_path_from_changed_record(record, kind).unwrap_or_default();
            let original_path =
                if matches!(kind, StatusEntryKind::Renamed | StatusEntryKind::Copied) {
                    let original = records.get(index + 1).map(|value| (*value).to_string());
                    index += 1;
                    original
                } else {
                    None
                };
            entries.push(StatusEntry {
                kind,
                xy: Some(xy.to_string()),
                path,
                original_path,
            });
            index += 1;
            continue;
        }

        if record.starts_with("u ") {
            counts.unmerged += 1;
            let path = parse_path_after_fields(record, 10).unwrap_or_default();
            entries.push(StatusEntry {
                kind: StatusEntryKind::Unmerged,
                xy: None,
                path,
                original_path: None,
            });
            index += 1;
            continue;
        }

        index += 1;
    }

    if head_oid.is_empty() {
        return Err(DevRelayError::GitCommand {
            cwd: PathBuf::from("."),
            args: "status --porcelain=v2 -z --branch".to_string(),
            stderr: "repository has no HEAD or status output was incomplete".to_string(),
        });
    }

    Ok(GitStatus {
        head_oid,
        branch,
        upstream,
        entries,
        counts,
    })
}

fn add_xy_counts(xy: &str, counts: &mut StatusCounts) {
    let mut chars = xy.chars();
    let x = chars.next().unwrap_or('.');
    let y = chars.next().unwrap_or('.');
    if x != '.' {
        counts.staged += 1;
    }
    if y != '.' {
        counts.unstaged += 1;
    }
}

fn changed_entry_kind(record: &str) -> StatusEntryKind {
    if !record.starts_with("2 ") {
        return StatusEntryKind::Ordinary;
    }
    match record.split_whitespace().nth(8) {
        Some(score) if score.starts_with('C') => StatusEntryKind::Copied,
        _ => StatusEntryKind::Renamed,
    }
}

fn parse_path_from_changed_record(record: &str, kind: StatusEntryKind) -> Option<String> {
    let fields_before_path = match kind {
        StatusEntryKind::Ordinary => 8,
        StatusEntryKind::Renamed | StatusEntryKind::Copied => 9,
        StatusEntryKind::Unmerged | StatusEntryKind::Untracked | StatusEntryKind::Ignored => {
            return None;
        }
    };
    parse_path_after_fields(record, fields_before_path)
}

fn parse_path_after_fields(record: &str, fields_before_path: usize) -> Option<String> {
    let mut parts = record.splitn(fields_before_path + 1, ' ');
    for _ in 0..fields_before_path {
        parts.next()?;
    }
    parts.next().map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_branch_headers_and_summary() {
        let raw = concat!(
            "# branch.oid abc\0",
            "# branch.head main\0",
            "# branch.upstream origin/main\0",
            "# branch.ab +1 -0\0",
            "1 .M N... 100644 100644 100644 a b src/lib.rs\0",
            "? notes.md\0",
        );
        let status = parse_status_porcelain_v2(raw).expect("status should parse");
        assert_eq!(status.head_oid, "abc");
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!(status.counts.unstaged, 1);
        assert_eq!(status.counts.untracked, 1);

        let summary = status.summary();
        assert_eq!(summary.branch.as_deref(), Some("main"));
        assert!(!summary.clean);
        assert!(!summary.initial);
    }

    #[test]
    fn parses_detached_head() {
        let raw = concat!(
            "# branch.oid abc\0",
            "# branch.head (detached)\0",
            "# branch.upstream origin/main\0",
        );
        let status = parse_status_porcelain_v2(raw).expect("status should parse");
        assert_eq!(status.head_oid, "abc");
        assert_eq!(status.branch, None);
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
    }

    #[test]
    fn parses_changed_entry_kinds_and_counts() {
        let raw = concat!(
            "# branch.oid abc\0",
            "# branch.head main\0",
            "1 A. N... 000000 100644 100644 0 a staged-add.rs\0",
            "1 M. N... 100644 100644 100644 a b staged-mod.rs\0",
            "1 D. N... 100644 000000 000000 a 0 staged-del.rs\0",
            "1 .M N... 100644 100644 100644 a a unstaged-mod.rs\0",
            "1 .D N... 100644 100644 000000 a a unstaged-del.rs\0",
        );
        let status = parse_status_porcelain_v2(raw).expect("status should parse");
        assert_eq!(status.counts.staged, 3);
        assert_eq!(status.counts.unstaged, 2);
        assert_eq!(
            status
                .entries
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "staged-add.rs",
                "staged-mod.rs",
                "staged-del.rs",
                "unstaged-mod.rs",
                "unstaged-del.rs"
            ]
        );
        assert!(
            status
                .entries
                .iter()
                .all(|entry| entry.kind == StatusEntryKind::Ordinary)
        );
    }

    #[test]
    fn parses_rename_and_copy_entries() {
        let raw = concat!(
            "# branch.oid abc\0",
            "# branch.head main\0",
            "2 R. N... 100644 100644 100644 a b R100 new name.rs\0",
            "old name.rs\0",
            "2 C. N... 100644 100644 100644 a b C100 copied.rs\0",
            "source.rs\0",
        );
        let status = parse_status_porcelain_v2(raw).expect("status should parse");
        assert_eq!(status.entries[0].kind, StatusEntryKind::Renamed);
        assert_eq!(status.entries[0].path, "new name.rs");
        assert_eq!(
            status.entries[0].original_path.as_deref(),
            Some("old name.rs")
        );
        assert_eq!(status.entries[1].kind, StatusEntryKind::Copied);
        assert_eq!(status.entries[1].path, "copied.rs");
        assert_eq!(
            status.entries[1].original_path.as_deref(),
            Some("source.rs")
        );
    }

    #[test]
    fn parses_untracked_ignored_and_unmerged_entries() {
        let raw = concat!(
            "# branch.oid abc\0",
            "# branch.head main\0",
            "? notes.md\0",
            "! target/debug/devrelay\0",
            "u UU N... 100644 100644 100644 100644 a b c conflicted path.rs\0",
        );
        let status = parse_status_porcelain_v2(raw).expect("status should parse");
        assert_eq!(status.counts.untracked, 1);
        assert_eq!(status.counts.ignored, 1);
        assert_eq!(status.counts.unmerged, 1);
        assert_eq!(status.entries[0].kind, StatusEntryKind::Untracked);
        assert_eq!(status.entries[1].kind, StatusEntryKind::Ignored);
        assert_eq!(status.entries[2].kind, StatusEntryKind::Unmerged);
        assert_eq!(status.entries[2].path, "conflicted path.rs");
    }

    #[test]
    fn preserves_valid_utf8_paths_from_nul_records() {
        let raw = concat!(
            "# branch.oid abc\0",
            "# branch.head main\0",
            "1 .M N... 100644 100644 100644 a b path with spaces.rs\0",
            "1 .M N... 100644 100644 100644 a b path\twith\ttabs.rs\0",
            "1 .M N... 100644 100644 100644 a b 유니코드.rs\0",
        );
        let status = parse_status_porcelain_v2(raw).expect("status should parse");
        assert_eq!(
            status
                .entries
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["path with spaces.rs", "path\twith\ttabs.rs", "유니코드.rs"]
        );
    }

    #[test]
    fn defines_initial_repository_behavior() {
        let raw = concat!(
            "# branch.oid (initial)\0",
            "# branch.head main\0",
            "? first.txt\0",
        );
        let status = parse_status_porcelain_v2(raw).expect("initial status should parse");
        assert_eq!(status.head_oid, "(initial)");
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert!(status.is_initial());
        assert!(status.summary().initial);
    }
}
