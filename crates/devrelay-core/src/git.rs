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
use std::path::{Path, PathBuf};
use std::process::Command;

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

impl GitStatus {
    pub fn is_clean(&self) -> bool {
        self.counts.staged == 0
            && self.counts.unstaged == 0
            && self.counts.untracked == 0
            && self.counts.unmerged == 0
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
        parse_status(&raw)
    }

    pub fn current_index_tree(&self) -> Result<String> {
        self.run(&["write-tree"])
    }
}

fn parse_status(raw: &str) -> Result<GitStatus> {
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
            let kind = if record.starts_with("2 ") {
                StatusEntryKind::Renamed
            } else {
                StatusEntryKind::Ordinary
            };
            let xy = record.split_whitespace().nth(1).ok_or_else(|| {
                DevRelayError::Manifest(format!("invalid git status record: {record}"))
            })?;
            add_xy_counts(xy, &mut counts);
            let path = parse_path_from_ordinary_record(record).unwrap_or_default();
            let original_path = if kind == StatusEntryKind::Renamed {
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
            let path = record
                .split(' ')
                .next_back()
                .unwrap_or_default()
                .to_string();
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

fn parse_path_from_ordinary_record(record: &str) -> Option<String> {
    let mut parts = record.splitn(9, ' ');
    for _ in 0..8 {
        parts.next()?;
    }
    parts.next().map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_v2_counts() {
        let raw = concat!(
            "# branch.oid abc\0",
            "# branch.head main\0",
            "1 .M N... 100644 100644 100644 a b src/lib.rs\0",
            "? notes.md\0",
        );
        let status = parse_status(raw).expect("status should parse");
        assert_eq!(status.head_oid, "abc");
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.counts.unstaged, 1);
        assert_eq!(status.counts.untracked, 1);
    }
}
