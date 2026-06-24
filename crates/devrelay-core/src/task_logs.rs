//! Task execution log buffering and disk spooling.
//!
//! Live log delivery is best-effort and bounded in memory. Durable retrieval
//! reads the redacted JSONL spool written for a task run.

use crate::{
    DevRelayError, DevRelayHome, LogRedactor, Result, TaskExecutionLogEvent, TaskExecutionLogSink,
    TaskExecutionLogStream,
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const TASK_LOGS_DIR: &str = "task-logs";
const DEFAULT_LIVE_BUFFER_BYTES: usize = 64 * 1024;
const DEFAULT_SPOOL_BYTES: u64 = 16 * 1024 * 1024;
pub const TASK_LOG_TRUNCATION_MARKER: &str = "[devrelay log truncated]\n";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskLogStoreConfig {
    pub live_buffer_bytes: usize,
    pub spool_bytes: u64,
}

impl Default for TaskLogStoreConfig {
    fn default() -> Self {
        Self {
            live_buffer_bytes: DEFAULT_LIVE_BUFFER_BYTES,
            spool_bytes: DEFAULT_SPOOL_BYTES,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskLogRecord {
    pub sequence: u64,
    pub stream: TaskExecutionLogStream,
    pub chunk: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskLogRetrieval {
    pub task_run_id: String,
    pub records: Vec<TaskLogRecord>,
    pub truncated: bool,
}

#[derive(Debug)]
pub struct TaskLogStore {
    task_run_id: String,
    path: PathBuf,
    config: TaskLogStoreConfig,
    redactor: LogRedactor,
    live_buffer: VecDeque<TaskLogRecord>,
    live_buffer_bytes: usize,
    spool_bytes: u64,
    next_sequence: u64,
    spool_truncated: bool,
}

impl TaskLogStore {
    pub fn open(
        home: &DevRelayHome,
        project_id: &str,
        task_run_id: &str,
        config: TaskLogStoreConfig,
    ) -> Result<Self> {
        validate_task_run_id(task_run_id)?;
        let path = task_log_spool_path(home, project_id, task_run_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)?;
        Ok(Self {
            task_run_id: task_run_id.to_string(),
            path,
            config,
            redactor: LogRedactor::new(),
            live_buffer: VecDeque::new(),
            live_buffer_bytes: 0,
            spool_bytes: 0,
            next_sequence: 0,
            spool_truncated: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn live_records(&self) -> Vec<TaskLogRecord> {
        self.live_buffer.iter().cloned().collect()
    }

    pub fn read_spool(&self) -> Result<TaskLogRetrieval> {
        read_task_log_spool(&self.path, &self.task_run_id)
    }

    fn append_record(&mut self, stream: TaskExecutionLogStream, chunk: &str) -> Result<()> {
        let redacted = self.redactor.redact_text(chunk);
        let live_record = self.live_record(stream, &redacted);
        self.push_live_record(live_record);

        if self.spool_truncated {
            return Ok(());
        }

        let record = TaskLogRecord {
            sequence: self.next_sequence,
            stream,
            chunk: redacted,
            truncated: false,
        };
        let encoded = encode_record(&record)?;
        if self.spool_bytes.saturating_add(encoded.len() as u64) > self.config.spool_bytes {
            self.write_truncation_marker(stream)?;
            return Ok(());
        }
        self.write_encoded_record(&encoded)?;
        self.next_sequence += 1;
        Ok(())
    }

    fn live_record(&self, stream: TaskExecutionLogStream, chunk: &str) -> TaskLogRecord {
        let max = self.config.live_buffer_bytes.max(1);
        let (chunk, truncated) = truncate_to_byte_limit(chunk, max);
        TaskLogRecord {
            sequence: self.next_sequence,
            stream,
            chunk,
            truncated,
        }
    }

    fn push_live_record(&mut self, record: TaskLogRecord) {
        let record_bytes = record.chunk.len();
        self.live_buffer_bytes = self.live_buffer_bytes.saturating_add(record_bytes);
        self.live_buffer.push_back(record);
        while self.live_buffer_bytes > self.config.live_buffer_bytes {
            let Some(removed) = self.live_buffer.pop_front() else {
                break;
            };
            self.live_buffer_bytes = self.live_buffer_bytes.saturating_sub(removed.chunk.len());
        }
    }

    fn write_truncation_marker(&mut self, stream: TaskExecutionLogStream) -> Result<()> {
        let record = TaskLogRecord {
            sequence: self.next_sequence,
            stream,
            chunk: TASK_LOG_TRUNCATION_MARKER.to_string(),
            truncated: true,
        };
        let encoded = encode_record(&record)?;
        self.write_encoded_record(&encoded)?;
        self.next_sequence += 1;
        self.spool_truncated = true;
        self.push_live_record(record);
        Ok(())
    }

    fn write_encoded_record(&mut self, encoded: &[u8]) -> Result<()> {
        let mut file = OpenOptions::new().append(true).open(&self.path)?;
        file.write_all(encoded)?;
        self.spool_bytes = self.spool_bytes.saturating_add(encoded.len() as u64);
        Ok(())
    }
}

impl TaskExecutionLogSink for TaskLogStore {
    fn on_log(&mut self, event: &TaskExecutionLogEvent) -> Result<()> {
        self.append_record(event.stream, &event.chunk)
    }
}

pub fn task_log_spool_path(home: &DevRelayHome, project_id: &str, task_run_id: &str) -> PathBuf {
    home.project_data_dir(project_id)
        .join(TASK_LOGS_DIR)
        .join(format!("{task_run_id}.jsonl"))
}

pub fn read_task_log_spool(path: impl AsRef<Path>, task_run_id: &str) -> Result<TaskLogRetrieval> {
    let file = OpenOptions::new().read(true).open(path)?;
    let mut records = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        records.push(serde_json::from_str::<TaskLogRecord>(&line)?);
    }
    Ok(TaskLogRetrieval {
        task_run_id: task_run_id.to_string(),
        truncated: records.iter().any(|record| record.truncated),
        records,
    })
}

fn encode_record(record: &TaskLogRecord) -> Result<Vec<u8>> {
    let mut encoded = serde_json::to_vec(record)?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn truncate_to_byte_limit(value: &str, limit: usize) -> (String, bool) {
    if value.len() <= limit {
        return (value.to_string(), false);
    }
    if limit <= TASK_LOG_TRUNCATION_MARKER.len() {
        let mut end = limit;
        while !TASK_LOG_TRUNCATION_MARKER.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        return (TASK_LOG_TRUNCATION_MARKER[..end].to_string(), true);
    }
    let mut end = limit
        .saturating_sub(TASK_LOG_TRUNCATION_MARKER.len())
        .max(1);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    (
        format!("{}{}", &value[..end], TASK_LOG_TRUNCATION_MARKER),
        true,
    )
}

fn validate_task_run_id(task_run_id: &str) -> Result<()> {
    if task_run_id.is_empty()
        || task_run_id.len() > 128
        || task_run_id.bytes().any(
            |byte| !matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.'),
        )
    {
        return Err(DevRelayError::Config(format!(
            "invalid task run id {task_run_id:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_log_store_buffers_live_events_and_spools_to_disk() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let mut store = TaskLogStore::open(
            &home,
            "12345678",
            "tr_logs",
            TaskLogStoreConfig {
                live_buffer_bytes: 16,
                spool_bytes: 4096,
            },
        )
        .unwrap();

        store
            .on_log(&TaskExecutionLogEvent {
                stream: TaskExecutionLogStream::Stdout,
                chunk: "first\n".to_string(),
            })
            .unwrap();
        store
            .on_log(&TaskExecutionLogEvent {
                stream: TaskExecutionLogStream::Stderr,
                chunk: "second\n".to_string(),
            })
            .unwrap();
        store
            .on_log(&TaskExecutionLogEvent {
                stream: TaskExecutionLogStream::Stdout,
                chunk: "third\n".to_string(),
            })
            .unwrap();

        assert_eq!(store.live_records().len(), 2);
        let retrieved = store.read_spool().unwrap();
        assert!(!retrieved.truncated);
        assert_eq!(retrieved.records.len(), 3);
        assert_eq!(retrieved.records[0].chunk, "first\n");
        assert_eq!(retrieved.records[1].stream, TaskExecutionLogStream::Stderr);
    }

    #[test]
    fn task_log_store_redacts_and_writes_truncation_marker() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let mut store = TaskLogStore::open(
            &home,
            "12345678",
            "tr_truncated",
            TaskLogStoreConfig {
                live_buffer_bytes: 1024,
                spool_bytes: 120,
            },
        )
        .unwrap();

        store
            .on_log(&TaskExecutionLogEvent {
                stream: TaskExecutionLogStream::Stdout,
                chunk: "TOKEN=super-secret-value\n".to_string(),
            })
            .unwrap();
        store
            .on_log(&TaskExecutionLogEvent {
                stream: TaskExecutionLogStream::Stdout,
                chunk: "x".repeat(512),
            })
            .unwrap();
        store
            .on_log(&TaskExecutionLogEvent {
                stream: TaskExecutionLogStream::Stdout,
                chunk: "ignored after truncation\n".to_string(),
            })
            .unwrap();

        let retrieved = store.read_spool().unwrap();
        assert!(retrieved.truncated);
        assert_eq!(retrieved.records[0].chunk, "TOKEN=<redacted>\n");
        assert!(
            retrieved
                .records
                .iter()
                .any(|record| record.chunk == TASK_LOG_TRUNCATION_MARKER && record.truncated)
        );
        assert!(
            !retrieved
                .records
                .iter()
                .any(|record| { record.chunk.contains("ignored after truncation") })
        );
    }

    #[test]
    fn live_buffer_truncates_large_single_chunks() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let mut store = TaskLogStore::open(
            &home,
            "12345678",
            "tr_live",
            TaskLogStoreConfig {
                live_buffer_bytes: 32,
                spool_bytes: 4096,
            },
        )
        .unwrap();

        store
            .on_log(&TaskExecutionLogEvent {
                stream: TaskExecutionLogStream::Stdout,
                chunk: "a".repeat(128),
            })
            .unwrap();

        let live = store.live_records();
        assert_eq!(live.len(), 1);
        assert!(live[0].truncated);
        assert!(live[0].chunk.len() <= 32);
    }

    #[test]
    fn rejects_unsafe_task_run_ids_for_spool_paths() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));

        let err = TaskLogStore::open(
            &home,
            "12345678",
            "../escape",
            TaskLogStoreConfig::default(),
        )
        .unwrap_err();

        assert!(err.to_string().contains("invalid task run id"));
    }
}
