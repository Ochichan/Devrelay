//! Durable crash journal for resumable local workflows.
//!
//! The journal is append-only JSONL. It records phase boundaries before and
//! after durable work so recovery code can replay the last known phase after a
//! process crash.

use crate::{DevRelayError, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum CrashJournalPhase {
    SnapshotCreationStart,
    SnapshotCreationComplete,
    PublishStart,
    PublishComplete,
    TargetApplyStart,
    TargetBackupComplete,
    BaseApplied,
    WorkApplied,
    IndexApplied,
    Verified,
    LeaseCommitted,
}

impl CrashJournalPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SnapshotCreationStart => "snapshot-creation-start",
            Self::SnapshotCreationComplete => "snapshot-creation-complete",
            Self::PublishStart => "publish-start",
            Self::PublishComplete => "publish-complete",
            Self::TargetApplyStart => "target-apply-start",
            Self::TargetBackupComplete => "target-backup-complete",
            Self::BaseApplied => "base-applied",
            Self::WorkApplied => "work-applied",
            Self::IndexApplied => "index-applied",
            Self::Verified => "verified",
            Self::LeaseCommitted => "lease-committed",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::SnapshotCreationComplete
                | Self::PublishComplete
                | Self::Verified
                | Self::LeaseCommitted
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrashJournalRecord {
    pub sequence_number: u64,
    pub operation_id: String,
    pub project_id: Option<String>,
    pub workspace_id: Option<String>,
    pub snapshot_id: Option<String>,
    pub lease_id: Option<String>,
    pub phase: CrashJournalPhase,
    pub detail_json: String,
    pub created_at_unix_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrashJournalOperationReplay {
    pub operation_id: String,
    pub records: Vec<CrashJournalRecord>,
    pub latest_phase: Option<CrashJournalPhase>,
    pub terminal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrashJournalReplay {
    pub operations: Vec<CrashJournalOperationReplay>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashJournalFaultPoint {
    BeforeAppend,
    AfterRecordWrite,
}

impl CrashJournalFaultPoint {
    fn as_str(self) -> &'static str {
        match self {
            Self::BeforeAppend => "before-append",
            Self::AfterRecordWrite => "after-record-write",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CrashJournal {
    path: PathBuf,
    fault: Option<CrashJournalFaultPoint>,
}

#[derive(Debug, Clone)]
struct CrashJournalRecordInput {
    operation_id: String,
    project_id: Option<String>,
    workspace_id: Option<String>,
    snapshot_id: Option<String>,
    lease_id: Option<String>,
    phase: CrashJournalPhase,
    detail: Value,
}

impl CrashJournal {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            fault: None,
        }
    }

    pub fn with_fault_injection(mut self, fault: CrashJournalFaultPoint) -> Self {
        self.fault = Some(fault);
        self
    }

    pub fn set_fault_injection(&mut self, fault: Option<CrashJournalFaultPoint>) {
        self.fault = fault;
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record_snapshot_creation_start(
        &self,
        operation_id: impl Into<String>,
        project_id: impl Into<String>,
        workspace_id: impl Into<String>,
    ) -> Result<CrashJournalRecord> {
        self.append_record(CrashJournalRecordInput {
            operation_id: operation_id.into(),
            project_id: Some(project_id.into()),
            workspace_id: Some(workspace_id.into()),
            snapshot_id: None,
            lease_id: None,
            phase: CrashJournalPhase::SnapshotCreationStart,
            detail: serde_json::json!({}),
        })
    }

    pub fn record_snapshot_creation_complete(
        &self,
        operation_id: impl Into<String>,
        project_id: impl Into<String>,
        workspace_id: impl Into<String>,
        snapshot_id: impl Into<String>,
    ) -> Result<CrashJournalRecord> {
        let snapshot_id = snapshot_id.into();
        self.append_record(CrashJournalRecordInput {
            operation_id: operation_id.into(),
            project_id: Some(project_id.into()),
            workspace_id: Some(workspace_id.into()),
            snapshot_id: Some(snapshot_id.clone()),
            lease_id: None,
            phase: CrashJournalPhase::SnapshotCreationComplete,
            detail: serde_json::json!({ "snapshot_id": snapshot_id }),
        })
    }

    pub fn record_publish_start(
        &self,
        operation_id: impl Into<String>,
        project_id: impl Into<String>,
        snapshot_id: impl Into<String>,
    ) -> Result<CrashJournalRecord> {
        let snapshot_id = snapshot_id.into();
        self.append_record(CrashJournalRecordInput {
            operation_id: operation_id.into(),
            project_id: Some(project_id.into()),
            workspace_id: None,
            snapshot_id: Some(snapshot_id.clone()),
            lease_id: None,
            phase: CrashJournalPhase::PublishStart,
            detail: serde_json::json!({ "snapshot_id": snapshot_id }),
        })
    }

    pub fn record_publish_complete(
        &self,
        operation_id: impl Into<String>,
        project_id: impl Into<String>,
        snapshot_id: impl Into<String>,
    ) -> Result<CrashJournalRecord> {
        let snapshot_id = snapshot_id.into();
        self.append_record(CrashJournalRecordInput {
            operation_id: operation_id.into(),
            project_id: Some(project_id.into()),
            workspace_id: None,
            snapshot_id: Some(snapshot_id.clone()),
            lease_id: None,
            phase: CrashJournalPhase::PublishComplete,
            detail: serde_json::json!({ "snapshot_id": snapshot_id }),
        })
    }

    pub fn record_target_apply_start(
        &self,
        operation_id: impl Into<String>,
        project_id: impl Into<String>,
        workspace_id: impl Into<String>,
        snapshot_id: impl Into<String>,
    ) -> Result<CrashJournalRecord> {
        let snapshot_id = snapshot_id.into();
        self.append_record(CrashJournalRecordInput {
            operation_id: operation_id.into(),
            project_id: Some(project_id.into()),
            workspace_id: Some(workspace_id.into()),
            snapshot_id: Some(snapshot_id.clone()),
            lease_id: None,
            phase: CrashJournalPhase::TargetApplyStart,
            detail: serde_json::json!({ "snapshot_id": snapshot_id }),
        })
    }

    pub fn record_target_backup_complete(
        &self,
        operation_id: impl Into<String>,
        project_id: impl Into<String>,
        workspace_id: impl Into<String>,
        snapshot_id: impl Into<String>,
    ) -> Result<CrashJournalRecord> {
        let snapshot_id = snapshot_id.into();
        self.append_record(CrashJournalRecordInput {
            operation_id: operation_id.into(),
            project_id: Some(project_id.into()),
            workspace_id: Some(workspace_id.into()),
            snapshot_id: Some(snapshot_id.clone()),
            lease_id: None,
            phase: CrashJournalPhase::TargetBackupComplete,
            detail: serde_json::json!({ "snapshot_id": snapshot_id }),
        })
    }

    pub fn record_base_applied(
        &self,
        operation_id: impl Into<String>,
        project_id: impl Into<String>,
        workspace_id: impl Into<String>,
        snapshot_id: impl Into<String>,
    ) -> Result<CrashJournalRecord> {
        self.record_apply_phase(
            operation_id,
            project_id,
            workspace_id,
            snapshot_id,
            CrashJournalPhase::BaseApplied,
        )
    }

    pub fn record_work_applied(
        &self,
        operation_id: impl Into<String>,
        project_id: impl Into<String>,
        workspace_id: impl Into<String>,
        snapshot_id: impl Into<String>,
    ) -> Result<CrashJournalRecord> {
        self.record_apply_phase(
            operation_id,
            project_id,
            workspace_id,
            snapshot_id,
            CrashJournalPhase::WorkApplied,
        )
    }

    pub fn record_index_applied(
        &self,
        operation_id: impl Into<String>,
        project_id: impl Into<String>,
        workspace_id: impl Into<String>,
        snapshot_id: impl Into<String>,
    ) -> Result<CrashJournalRecord> {
        self.record_apply_phase(
            operation_id,
            project_id,
            workspace_id,
            snapshot_id,
            CrashJournalPhase::IndexApplied,
        )
    }

    pub fn record_verified(
        &self,
        operation_id: impl Into<String>,
        project_id: impl Into<String>,
        workspace_id: impl Into<String>,
        snapshot_id: impl Into<String>,
    ) -> Result<CrashJournalRecord> {
        self.record_apply_phase(
            operation_id,
            project_id,
            workspace_id,
            snapshot_id,
            CrashJournalPhase::Verified,
        )
    }

    pub fn record_lease_committed(
        &self,
        operation_id: impl Into<String>,
        project_id: impl Into<String>,
        workspace_id: impl Into<String>,
        snapshot_id: impl Into<String>,
        lease_id: impl Into<String>,
    ) -> Result<CrashJournalRecord> {
        let snapshot_id = snapshot_id.into();
        let lease_id = lease_id.into();
        self.append_record(CrashJournalRecordInput {
            operation_id: operation_id.into(),
            project_id: Some(project_id.into()),
            workspace_id: Some(workspace_id.into()),
            snapshot_id: Some(snapshot_id.clone()),
            lease_id: Some(lease_id.clone()),
            phase: CrashJournalPhase::LeaseCommitted,
            detail: serde_json::json!({
                "snapshot_id": snapshot_id,
                "lease_id": lease_id,
            }),
        })
    }

    pub fn replay(&self) -> Result<CrashJournalReplay> {
        let mut by_operation = BTreeMap::<String, Vec<CrashJournalRecord>>::new();
        for record in self.records()? {
            by_operation
                .entry(record.operation_id.clone())
                .or_default()
                .push(record);
        }

        let operations = by_operation
            .into_iter()
            .map(|(operation_id, mut records)| {
                records.sort_by_key(|record| record.sequence_number);
                let latest_phase = records.last().map(|record| record.phase);
                CrashJournalOperationReplay {
                    operation_id,
                    records,
                    latest_phase,
                    terminal: latest_phase.is_some_and(CrashJournalPhase::is_terminal),
                }
            })
            .collect();
        Ok(CrashJournalReplay { operations })
    }

    pub fn cleanup_completed(&self) -> Result<usize> {
        let replay = self.replay()?;
        let mut retained = Vec::new();
        let mut removed = 0_usize;
        for operation in replay.operations {
            if operation.terminal {
                removed += operation.records.len();
            } else {
                retained.extend(operation.records);
            }
        }

        self.rewrite_records(&retained)?;
        Ok(removed)
    }

    fn record_apply_phase(
        &self,
        operation_id: impl Into<String>,
        project_id: impl Into<String>,
        workspace_id: impl Into<String>,
        snapshot_id: impl Into<String>,
        phase: CrashJournalPhase,
    ) -> Result<CrashJournalRecord> {
        let snapshot_id = snapshot_id.into();
        self.append_record(CrashJournalRecordInput {
            operation_id: operation_id.into(),
            project_id: Some(project_id.into()),
            workspace_id: Some(workspace_id.into()),
            snapshot_id: Some(snapshot_id.clone()),
            lease_id: None,
            phase,
            detail: serde_json::json!({ "snapshot_id": snapshot_id }),
        })
    }

    fn append_record(&self, input: CrashJournalRecordInput) -> Result<CrashJournalRecord> {
        inject_fault(self.fault, CrashJournalFaultPoint::BeforeAppend)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let record = CrashJournalRecord {
            sequence_number: self.next_sequence_number()?,
            operation_id: input.operation_id,
            project_id: input.project_id,
            workspace_id: input.workspace_id,
            snapshot_id: input.snapshot_id,
            lease_id: input.lease_id,
            phase: input.phase,
            detail_json: serde_json::to_string(&input.detail)?,
            created_at_unix_seconds: unix_now_seconds(),
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(serde_json::to_string(&record)?.as_bytes())?;
        file.write_all(b"\n")?;
        inject_fault(self.fault, CrashJournalFaultPoint::AfterRecordWrite)?;
        file.sync_all()?;
        Ok(record)
    }

    fn next_sequence_number(&self) -> Result<u64> {
        Ok(self
            .records()?
            .last()
            .map(|record| record.sequence_number.saturating_add(1))
            .unwrap_or(1))
    }

    fn records(&self) -> Result<Vec<CrashJournalRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for (line_index, line) in reader.lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let record = serde_json::from_str::<CrashJournalRecord>(&line).map_err(|err| {
                DevRelayError::Config(format!(
                    "invalid crash journal line {} in {}: {err}",
                    line_index + 1,
                    self.path.display()
                ))
            })?;
            records.push(record);
        }
        records.sort_by_key(|record| record.sequence_number);
        Ok(records)
    }

    fn rewrite_records(&self, records: &[CrashJournalRecord]) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp_path = self.path.with_extension("tmp");
        {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&temp_path)?;
            for record in records {
                file.write_all(serde_json::to_string(record)?.as_bytes())?;
                file.write_all(b"\n")?;
            }
            file.sync_all()?;
        }
        fs::rename(temp_path, &self.path)?;
        Ok(())
    }
}

fn inject_fault(
    configured: Option<CrashJournalFaultPoint>,
    fault: CrashJournalFaultPoint,
) -> Result<()> {
    if configured == Some(fault) {
        return Err(DevRelayError::Config(format!(
            "injected crash journal fault at {}",
            fault.as_str()
        )));
    }
    Ok(())
}

fn unix_now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phases(records: &[CrashJournalRecord]) -> Vec<CrashJournalPhase> {
        records.iter().map(|record| record.phase).collect()
    }

    #[test]
    fn crash_journal_records_replays_and_cleans_completed_operations() {
        let temp = tempfile::tempdir().unwrap();
        let journal = CrashJournal::open(temp.path().join("crash.jsonl"));

        journal
            .record_snapshot_creation_start("op-snapshot", "project123", "w-source")
            .unwrap();
        journal
            .record_snapshot_creation_complete(
                "op-snapshot",
                "project123",
                "w-source",
                "s1_snapshot",
            )
            .unwrap();
        journal
            .record_publish_start("op-publish", "project123", "s1_snapshot")
            .unwrap();

        let replay = journal.replay().unwrap();
        assert_eq!(replay.operations.len(), 2);
        let snapshot_operation = replay
            .operations
            .iter()
            .find(|operation| operation.operation_id == "op-snapshot")
            .unwrap();
        let publish_operation = replay
            .operations
            .iter()
            .find(|operation| operation.operation_id == "op-publish")
            .unwrap();
        assert!(snapshot_operation.terminal);
        assert_eq!(
            publish_operation.latest_phase,
            Some(CrashJournalPhase::PublishStart)
        );
        assert!(!publish_operation.terminal);

        let removed = journal.cleanup_completed().unwrap();
        assert_eq!(removed, 2);
        let replay = journal.replay().unwrap();
        assert_eq!(replay.operations.len(), 1);
        assert_eq!(replay.operations[0].operation_id, "op-publish");
        assert_eq!(replay.operations[0].records.len(), 1);
    }

    #[test]
    fn crash_journal_records_all_apply_and_handoff_phases() {
        let temp = tempfile::tempdir().unwrap();
        let journal = CrashJournal::open(temp.path().join("crash.jsonl"));

        journal
            .record_target_apply_start("op-apply", "project123", "w-target", "s1_snapshot")
            .unwrap();
        journal
            .record_target_backup_complete("op-apply", "project123", "w-target", "s1_snapshot")
            .unwrap();
        journal
            .record_base_applied("op-apply", "project123", "w-target", "s1_snapshot")
            .unwrap();
        journal
            .record_work_applied("op-apply", "project123", "w-target", "s1_snapshot")
            .unwrap();
        journal
            .record_index_applied("op-apply", "project123", "w-target", "s1_snapshot")
            .unwrap();
        journal
            .record_verified("op-apply", "project123", "w-target", "s1_snapshot")
            .unwrap();
        journal
            .record_lease_committed(
                "op-apply",
                "project123",
                "w-target",
                "s1_snapshot",
                "lease123",
            )
            .unwrap();

        let replay = journal.replay().unwrap();
        assert_eq!(
            phases(&replay.operations[0].records),
            vec![
                CrashJournalPhase::TargetApplyStart,
                CrashJournalPhase::TargetBackupComplete,
                CrashJournalPhase::BaseApplied,
                CrashJournalPhase::WorkApplied,
                CrashJournalPhase::IndexApplied,
                CrashJournalPhase::Verified,
                CrashJournalPhase::LeaseCommitted,
            ]
        );
        assert!(replay.operations[0].terminal);
        assert_eq!(
            replay.operations[0].latest_phase,
            Some(CrashJournalPhase::LeaseCommitted)
        );
    }

    #[test]
    fn crash_journal_fault_after_record_write_is_replayable() {
        let temp = tempfile::tempdir().unwrap();
        let journal = CrashJournal::open(temp.path().join("crash.jsonl"))
            .with_fault_injection(CrashJournalFaultPoint::AfterRecordWrite);

        let err = journal
            .record_publish_start("op-publish", "project123", "s1_snapshot")
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("injected crash journal fault at after-record-write")
        );
        let replay = CrashJournal::open(journal.path()).replay().unwrap();
        assert_eq!(replay.operations.len(), 1);
        assert_eq!(
            replay.operations[0].latest_phase,
            Some(CrashJournalPhase::PublishStart)
        );
        assert!(!replay.operations[0].terminal);
    }
}
