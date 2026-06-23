//! Structured log records, redaction, and file rotation helpers.
//!
//! Logs are diagnostic data. They should remain useful for local debugging while
//! avoiding accidental disclosure of secrets, credentialed remotes, and local
//! paths when diagnostics request path redaction.

use crate::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const REDACTED: &str = "<redacted>";
const REDACTED_PATH: &str = "<path>";
const SECRET_KEY_FRAGMENTS: &[&str] = &[
    "api_key",
    "apikey",
    "auth",
    "credential",
    "password",
    "passwd",
    "private_key",
    "secret",
    "token",
];
const URL_SCHEMES: &[&str] = &["https://", "http://", "ssh://"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StructuredLogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl StructuredLogLevel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuredLogFormat {
    JsonLine,
    Human,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredLogRecord {
    pub timestamp_unix_millis: u64,
    pub level: StructuredLogLevel,
    pub target: String,
    pub message: String,
    pub request_id: Option<String>,
    pub operation_id: Option<String>,
    pub fields: BTreeMap<String, String>,
}

impl StructuredLogRecord {
    pub fn new(
        level: StructuredLogLevel,
        target: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new_at(unix_millis(), level, target, message)
    }

    pub fn new_at(
        timestamp_unix_millis: u64,
        level: StructuredLogLevel,
        target: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            timestamp_unix_millis,
            level,
            target: target.into(),
            message: message.into(),
            request_id: None,
            operation_id: None,
            fields: BTreeMap::new(),
        }
    }

    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    pub fn with_operation_id(mut self, operation_id: impl Into<String>) -> Self {
        self.operation_id = Some(operation_id.into());
        self
    }

    pub fn with_field(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.insert(key.into(), value.into());
        self
    }

    pub fn to_json_line(&self, redactor: &LogRedactor) -> Result<String> {
        Ok(serde_json::to_string(&self.to_json_value(redactor))?)
    }

    pub fn to_human_line(&self, redactor: &LogRedactor) -> String {
        let mut parts = vec![
            self.timestamp_unix_millis.to_string(),
            self.level.as_str().to_string(),
            self.target.clone(),
            redactor.redact_text(&self.message),
        ];
        if let Some(request_id) = &self.request_id {
            parts.push(format!("request_id={}", redactor.redact_text(request_id)));
        }
        if let Some(operation_id) = &self.operation_id {
            parts.push(format!(
                "operation_id={}",
                redactor.redact_text(operation_id)
            ));
        }
        for (key, value) in &self.fields {
            parts.push(format!("{}={}", key, redactor.redact_field(key, value)));
        }
        parts.join(" ")
    }

    fn to_json_value(&self, redactor: &LogRedactor) -> Value {
        let fields = self
            .fields
            .iter()
            .map(|(key, value)| (key.clone(), redactor.redact_field(key, value)))
            .collect::<BTreeMap<_, _>>();
        json!({
            "timestamp_unix_millis": self.timestamp_unix_millis,
            "level": self.level.as_str(),
            "target": self.target,
            "message": redactor.redact_text(&self.message),
            "request_id": self.request_id.as_ref().map(|value| redactor.redact_text(value)),
            "operation_id": self.operation_id.as_ref().map(|value| redactor.redact_text(value)),
            "fields": fields,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogRedactor {
    redact_local_paths: bool,
    local_paths: Vec<PathBuf>,
}

impl LogRedactor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn for_diagnostics(local_paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut redactor = Self {
            redact_local_paths: true,
            local_paths: local_paths.into_iter().collect(),
        };
        redactor
            .local_paths
            .sort_by_key(|path| std::cmp::Reverse(path.as_os_str().len()));
        redactor
    }

    pub fn redact_field(&self, key: &str, value: &str) -> String {
        if is_secret_key(key) {
            REDACTED.to_string()
        } else {
            self.redact_text(value)
        }
    }

    pub fn redact_text(&self, value: &str) -> String {
        let value = redact_credentialed_urls(value);
        let value = redact_raw_secret_tokens(&value);
        let value = redact_secret_assignments(&value);
        if self.redact_local_paths {
            self.redact_paths(&value)
        } else {
            value
        }
    }

    pub fn redact_json_value(&self, value: Value) -> Value {
        match value {
            Value::String(value) => Value::String(self.redact_text(&value)),
            Value::Array(values) => Value::Array(
                values
                    .into_iter()
                    .map(|value| self.redact_json_value(value))
                    .collect(),
            ),
            Value::Object(values) => Value::Object(
                values
                    .into_iter()
                    .map(|(key, value)| {
                        let value = match value {
                            Value::String(value) => Value::String(self.redact_field(&key, &value)),
                            value => self.redact_json_value(value),
                        };
                        (key, value)
                    })
                    .collect(),
            ),
            value => value,
        }
    }

    fn redact_paths(&self, value: &str) -> String {
        let mut redacted = value.to_string();
        for path in &self.local_paths {
            let path = path.to_string_lossy();
            if !path.is_empty() {
                redacted = redacted.replace(path.as_ref(), REDACTED_PATH);
            }
        }
        redacted
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogRotation {
    pub max_bytes: u64,
    pub max_files: usize,
}

impl Default for LogRotation {
    fn default() -> Self {
        Self {
            max_bytes: 10 * 1024 * 1024,
            max_files: 5,
        }
    }
}

#[derive(Debug, Clone)]
pub struct StructuredLogFile {
    path: PathBuf,
    format: StructuredLogFormat,
    redactor: LogRedactor,
    rotation: LogRotation,
}

impl StructuredLogFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            format: StructuredLogFormat::JsonLine,
            redactor: LogRedactor::new(),
            rotation: LogRotation::default(),
        }
    }

    pub fn with_format(mut self, format: StructuredLogFormat) -> Self {
        self.format = format;
        self
    }

    pub fn with_redactor(mut self, redactor: LogRedactor) -> Self {
        self.redactor = redactor;
        self
    }

    pub fn with_rotation(mut self, rotation: LogRotation) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn append(&self, record: &StructuredLogRecord) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut line = match self.format {
            StructuredLogFormat::JsonLine => record.to_json_line(&self.redactor)?,
            StructuredLogFormat::Human => record.to_human_line(&self.redactor),
        };
        line.push('\n');
        self.rotate_if_needed(line.len() as u64)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(line.as_bytes())?;
        Ok(())
    }

    fn rotate_if_needed(&self, incoming_bytes: u64) -> Result<()> {
        if self.rotation.max_bytes == 0 {
            return Ok(());
        }
        let current_bytes = match fs::metadata(&self.path) {
            Ok(metadata) => metadata.len(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err.into()),
        };
        if current_bytes == 0 || current_bytes + incoming_bytes <= self.rotation.max_bytes {
            return Ok(());
        }
        rotate_log_files(&self.path, self.rotation.max_files)
    }
}

fn rotate_log_files(path: &Path, max_files: usize) -> Result<()> {
    if max_files == 0 {
        if path.exists() {
            fs::remove_file(path)?;
        }
        return Ok(());
    }

    let oldest = rotated_path(path, max_files);
    if oldest.exists() {
        fs::remove_file(&oldest)?;
    }
    for index in (1..max_files).rev() {
        let from = rotated_path(path, index);
        if from.exists() {
            fs::rename(from, rotated_path(path, index + 1))?;
        }
    }
    if path.exists() {
        fs::rename(path, rotated_path(path, 1))?;
    }
    Ok(())
}

fn rotated_path(path: &Path, index: usize) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(format!(".{index}"));
    PathBuf::from(value)
}

fn redact_credentialed_urls(value: &str) -> String {
    let mut output = String::new();
    let mut index = 0;
    while index < value.len() {
        let remainder = &value[index..];
        let Some((scheme_offset, scheme)) = find_next_scheme(remainder) else {
            output.push_str(remainder);
            break;
        };
        let start = index + scheme_offset;
        output.push_str(&value[index..start]);
        let url_end = find_url_end(value, start);
        let url = &value[start..url_end];
        output.push_str(&redact_url_userinfo(url, scheme));
        index = url_end;
    }
    output
}

fn redact_raw_secret_tokens(value: &str) -> String {
    let mut redacted = value.to_string();
    for token in raw_secret_tokens(value) {
        redacted = redacted.replace(token, REDACTED);
    }
    redacted
}

fn raw_secret_tokens(value: &str) -> Vec<&str> {
    token_candidates(value)
        .filter(|token| looks_like_raw_secret_token(token))
        .collect()
}

fn token_candidates(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(|ch: char| {
            !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | '+' | '='))
        })
        .filter(|candidate| !candidate.is_empty())
}

fn looks_like_raw_secret_token(token: &str) -> bool {
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

fn find_next_scheme(value: &str) -> Option<(usize, &'static str)> {
    URL_SCHEMES
        .iter()
        .filter_map(|scheme| value.find(scheme).map(|offset| (offset, *scheme)))
        .min_by_key(|(offset, _)| *offset)
}

fn find_url_end(value: &str, start: usize) -> usize {
    value[start..]
        .find(|ch: char| ch.is_whitespace() || matches!(ch, '"' | '\'' | ')' | ']'))
        .map(|offset| start + offset)
        .unwrap_or(value.len())
}

fn redact_url_userinfo(url: &str, scheme: &str) -> String {
    let authority_start = scheme.len();
    let authority_end = url[authority_start..]
        .find('/')
        .map(|offset| authority_start + offset)
        .unwrap_or(url.len());
    let authority = &url[authority_start..authority_end];
    let Some(at_offset) = authority.rfind('@') else {
        return url.to_string();
    };
    format!(
        "{}{}{}",
        &url[..authority_start],
        REDACTED,
        &url[authority_start + at_offset..]
    )
}

fn redact_secret_assignments(value: &str) -> String {
    let mut redacted = value.to_string();
    for key in SECRET_KEY_FRAGMENTS {
        redacted = redact_assignment_prefix(&redacted, &format!("{key}="));
        redacted = redact_assignment_prefix(&redacted, &format!("{key}:"));
    }
    redacted
}

fn redact_assignment_prefix(value: &str, prefix: &str) -> String {
    let mut output = String::new();
    let lower = value.to_ascii_lowercase();
    let mut index = 0;
    while let Some(relative) = lower[index..].find(prefix) {
        let prefix_start = index + relative;
        let value_start = prefix_start + prefix.len();
        output.push_str(&value[index..value_start]);
        let mut redact_start = value_start;
        while redact_start < value.len() {
            let Some(ch) = value[redact_start..].chars().next() else {
                break;
            };
            if !ch.is_whitespace() {
                break;
            }
            output.push(ch);
            redact_start += ch.len_utf8();
        }
        let mut redact_end = redact_start;
        while redact_end < value.len() {
            let Some(ch) = value[redact_end..].chars().next() else {
                break;
            };
            if ch.is_whitespace() || matches!(ch, ',' | ';' | '&') {
                break;
            }
            redact_end += ch.len_utf8();
        }
        if redact_end > redact_start {
            output.push_str(REDACTED);
        }
        index = redact_end;
    }
    output.push_str(&value[index..]);
    output
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    SECRET_KEY_FRAGMENTS
        .iter()
        .any(|fragment| key.contains(fragment))
}

fn unix_millis() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX));
    millis as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_json_lines_with_ids_and_redacted_fields() {
        let redactor = LogRedactor::for_diagnostics([PathBuf::from("/Users/me/work/project")]);
        let record = StructuredLogRecord::new_at(
            1_700_000_000_123,
            StructuredLogLevel::Info,
            "agent.rpc",
            "opened /Users/me/work/project with password=hunter2",
        )
        .with_request_id("req-1")
        .with_operation_id("op-1")
        .with_field("api_token", "secret-token")
        .with_field(
            "remote",
            "https://user:secret@github.com/example/devrelay.git",
        );

        let line = record.to_json_line(&redactor).unwrap();
        assert!(!line.ends_with('\n'));
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["timestamp_unix_millis"], 1_700_000_000_123u64);
        assert_eq!(value["level"], "info");
        assert_eq!(value["request_id"], "req-1");
        assert_eq!(value["operation_id"], "op-1");
        assert_eq!(value["message"], "opened <path> with password=<redacted>");
        assert_eq!(value["fields"]["api_token"], REDACTED);
        assert_eq!(
            value["fields"]["remote"],
            "https://<redacted>@github.com/example/devrelay.git"
        );
    }

    #[test]
    fn formats_human_dev_lines_with_ids() {
        let record = StructuredLogRecord::new_at(
            42,
            StructuredLogLevel::Warn,
            "agent.ipc",
            "retrying request",
        )
        .with_request_id("req-2")
        .with_operation_id("op-2")
        .with_field("attempt", "2");

        let line = record.to_human_line(&LogRedactor::new());
        assert!(line.contains("42 warn agent.ipc retrying request"));
        assert!(line.contains("request_id=req-2"));
        assert!(line.contains("operation_id=op-2"));
        assert!(line.contains("attempt=2"));
    }

    #[test]
    fn redacts_secret_values_and_credentialed_urls() {
        let redactor = LogRedactor::new();

        assert_eq!(redactor.redact_field("password", "hunter2"), REDACTED);
        assert_eq!(
            redactor.redact_text("token: abc123 and api_key=xyz"),
            "token: <redacted> and api_key=<redacted>"
        );
        assert_eq!(
            redactor.redact_text("raw ghp_abcdefghijklmnopqrstuvwxyz1234567890 token"),
            "raw <redacted> token"
        );
        assert_eq!(
            redactor.redact_text("aws key AKIA1234567890ABCDEF seen"),
            "aws key <redacted> seen"
        );
        assert_eq!(
            redactor.redact_text("provider key sk-abcdefghijklmnopqrstuvwxyz1234567890ABCDE"),
            "provider key <redacted>"
        );
        assert_eq!(
            redactor.redact_text("fetch https://token:secret@example.com/repo.git"),
            "fetch https://<redacted>@example.com/repo.git"
        );
        assert_eq!(
            redactor.redact_text("fetch https://example.com/repo.git"),
            "fetch https://example.com/repo.git"
        );
    }

    #[test]
    fn diagnostic_mode_redacts_configured_local_paths() {
        let redactor = LogRedactor::for_diagnostics([
            PathBuf::from("/Users/me/project"),
            PathBuf::from("/Users/me"),
        ]);

        assert_eq!(
            redactor.redact_text("open /Users/me/project/src/main.rs"),
            "open <path>/src/main.rs"
        );
    }

    #[test]
    fn redacts_nested_json_values() {
        let redactor = LogRedactor::for_diagnostics([PathBuf::from("/Users/me/project")]);
        let value = serde_json::json!({
            "path": "/Users/me/project/src/main.rs",
            "nested": {
                "token": "secret-token",
                "raw": "ghp_abcdefghijklmnopqrstuvwxyz1234567890",
                "remote": "https://user:secret@example.com/repo.git"
            }
        });

        let redacted = redactor.redact_json_value(value);

        assert_eq!(redacted["path"], "<path>/src/main.rs");
        assert_eq!(redacted["nested"]["token"], "<redacted>");
        assert_eq!(redacted["nested"]["raw"], "<redacted>");
        assert_eq!(
            redacted["nested"]["remote"],
            "https://<redacted>@example.com/repo.git"
        );
    }

    #[test]
    fn rotates_and_retains_log_files() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("agent.log");
        let writer = StructuredLogFile::new(&path).with_rotation(LogRotation {
            max_bytes: 180,
            max_files: 2,
        });

        for index in 0..6 {
            writer
                .append(
                    &StructuredLogRecord::new_at(
                        index,
                        StructuredLogLevel::Info,
                        "agent.test",
                        format!("message {index} {}", "x".repeat(80)),
                    )
                    .with_operation_id(format!("op-{index}")),
                )
                .unwrap();
        }

        assert!(path.exists());
        assert!(rotated_path(&path, 1).exists());
        assert!(rotated_path(&path, 2).exists());
        assert!(!rotated_path(&path, 3).exists());
        let current = fs::read_to_string(&path).unwrap();
        assert!(current.contains("op-5"));
    }
}
