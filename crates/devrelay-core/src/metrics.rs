//! Local-only operational metrics derived from durable metadata.
//!
//! Metrics are intentionally aggregated from DevRelay's local metadata DBs and
//! hydration state files. They do not include source files, snapshot objects, or
//! raw logs.

use crate::{
    AuditEventRecord, AuditEventType, AuditOutcome, DevRelayHome, HandoffJournalPhase,
    HandoffJournalRecord, HandoffRecord, HandoffState, HydrationState, HydrationStateRecord,
    LogRedactor, MetadataDb, Result, StoredSnapshot, TaskRunRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;

pub const LOCAL_METRICS_SCHEMA_VERSION: u32 = 1;
pub const LOCAL_METRICS_RECORD_LIMIT: usize = 1_000;

#[derive(Debug, Clone, PartialEq)]
pub struct LocalMetricsInput {
    pub generated_at_unix_seconds: u64,
    pub project: Option<String>,
    pub redacted: bool,
    pub audits: Vec<AuditEventRecord>,
    pub snapshots: Vec<StoredSnapshot>,
    pub handoffs: Vec<LocalMetricsHandoffInput>,
    pub task_runs: Vec<TaskRunRecord>,
    pub hydration: Vec<HydrationStateRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalMetricsHandoffInput {
    pub record: HandoffRecord,
    pub journal: Vec<HandoffJournalRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalMetricsReport {
    pub schema_version: u32,
    pub generated_at_unix_seconds: u64,
    pub project: Option<String>,
    pub privacy: LocalMetricsPrivacy,
    pub record_limit: usize,
    pub record_counts: LocalMetricsRecordCounts,
    pub continuation: VerifiedContinuationMetrics,
    pub checkpoints: CheckpointMetrics,
    pub apply: ApplyVerificationMetrics,
    pub handoffs: HandoffMetrics,
    pub environment: EnvironmentHydrationMetrics,
    pub scheduler: SchedulerChoiceMetrics,
    pub recording_gaps: Vec<MetricRecordingGap>,
}

impl LocalMetricsReport {
    pub fn redact(mut self, redactor: &LogRedactor) -> Self {
        for reason in &mut self.checkpoints.failure_reasons {
            reason.reason = redactor.redact_text(&reason.reason);
        }
        for reason in &mut self.apply.verification_failure_reasons {
            reason.reason = redactor.redact_text(&reason.reason);
        }
        for reason in &mut self.scheduler.choice_reasons {
            reason.reason = redactor.redact_text(&reason.reason);
            if let Some(explanation) = &mut reason.explanation {
                *explanation = redactor.redact_text(explanation);
            }
        }
        for gap in &mut self.recording_gaps {
            gap.reason = redactor.redact_text(&gap.reason);
        }
        self.privacy.redacted = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalMetricsPrivacy {
    pub local_by_default: bool,
    pub redacted: bool,
    pub source_code_included: bool,
    pub snapshot_objects_included: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalMetricsRecordCounts {
    pub audit_events: usize,
    pub snapshots: usize,
    pub handoffs: usize,
    pub handoff_journal_entries: usize,
    pub task_runs: usize,
    pub hydration_records: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedContinuationMetrics {
    pub handoff_attempts: usize,
    pub verified_attempts: usize,
    pub successes: usize,
    pub aborted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointMetrics {
    pub successes: usize,
    pub failure_reasons: Vec<MetricReasonCount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyVerificationMetrics {
    pub successful_applies: usize,
    pub verification_failures: usize,
    pub verification_failure_reasons: Vec<MetricReasonCount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffMetrics {
    pub phase_durations: Vec<HandoffPhaseDurationMetric>,
    pub committed_total_durations: Vec<HandoffTotalDurationMetric>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffPhaseDurationMetric {
    pub project_id: String,
    pub handoff_id: String,
    pub from_phase: HandoffJournalPhase,
    pub to_phase: HandoffJournalPhase,
    pub duration_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffTotalDurationMetric {
    pub project_id: String,
    pub handoff_id: String,
    pub duration_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentHydrationMetrics {
    pub records: usize,
    pub shell_ready: usize,
    pub app_ready: usize,
    pub failed: usize,
    pub duration_samples: Vec<EnvironmentHydrationDurationMetric>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentHydrationDurationMetric {
    pub project_id: String,
    pub workspace_id: Option<String>,
    pub duration_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerChoiceMetrics {
    pub task_runs_with_choice_reason: usize,
    pub task_runs_missing_choice_reason: usize,
    pub choice_reasons: Vec<SchedulerChoiceReasonCount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerChoiceReasonCount {
    pub reason: String,
    pub explanation: Option<String>,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricReasonCount {
    pub reason: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricRecordingGap {
    pub metric: String,
    pub reason: String,
}

pub fn collect_local_metrics_report(
    home: &DevRelayHome,
    project: Option<String>,
    project_ids: &[String],
    generated_at_unix_seconds: u64,
    redacted: bool,
) -> Result<LocalMetricsReport> {
    let mut input = LocalMetricsInput {
        generated_at_unix_seconds,
        project,
        redacted,
        audits: Vec::new(),
        snapshots: Vec::new(),
        handoffs: Vec::new(),
        task_runs: Vec::new(),
        hydration: Vec::new(),
    };

    for project_id in project_ids {
        let db_path = home.metadata_db_path(project_id);
        if db_path.exists() {
            let db = MetadataDb::open(&db_path)?;
            input
                .audits
                .extend(db.list_audit_events(Some(project_id), LOCAL_METRICS_RECORD_LIMIT)?);
            input
                .snapshots
                .extend(db.list_stored_snapshots(Some(project_id))?);
            for handoff in db.list_handoffs(Some(project_id))? {
                let journal = db.list_handoff_journal(&handoff.handoff_id)?;
                input.handoffs.push(LocalMetricsHandoffInput {
                    record: handoff,
                    journal,
                });
            }
            input
                .task_runs
                .extend(db.list_task_runs(Some(project_id), LOCAL_METRICS_RECORD_LIMIT)?);
        }
        input
            .hydration
            .extend(load_project_hydration_records(home, project_id)?);
    }

    Ok(build_local_metrics_report(input))
}

pub fn build_local_metrics_report(input: LocalMetricsInput) -> LocalMetricsReport {
    let handoff_journal_entries = input
        .handoffs
        .iter()
        .map(|handoff| handoff.journal.len())
        .sum();
    let record_counts = LocalMetricsRecordCounts {
        audit_events: input.audits.len(),
        snapshots: input.snapshots.len(),
        handoffs: input.handoffs.len(),
        handoff_journal_entries,
        task_runs: input.task_runs.len(),
        hydration_records: input.hydration.len(),
    };

    let continuation = continuation_metrics(&input.handoffs);
    let checkpoints = checkpoint_metrics(&input.snapshots, &input.audits);
    let apply = apply_metrics(&input.audits);
    let handoffs = handoff_metrics(&input.handoffs);
    let environment = environment_metrics(&input.hydration);
    let scheduler = scheduler_metrics(&input.task_runs);
    let mut recording_gaps = Vec::new();
    if environment.records > 0 && environment.duration_samples.is_empty() {
        recording_gaps.push(MetricRecordingGap {
            metric: "environment_hydrate_duration".to_string(),
            reason: "hydration state records do not yet store start and finish timestamps"
                .to_string(),
        });
    }

    LocalMetricsReport {
        schema_version: LOCAL_METRICS_SCHEMA_VERSION,
        generated_at_unix_seconds: input.generated_at_unix_seconds,
        project: input.project,
        privacy: LocalMetricsPrivacy {
            local_by_default: true,
            redacted: input.redacted,
            source_code_included: false,
            snapshot_objects_included: false,
        },
        record_limit: LOCAL_METRICS_RECORD_LIMIT,
        record_counts,
        continuation,
        checkpoints,
        apply,
        handoffs,
        environment,
        scheduler,
        recording_gaps,
    }
}

fn load_project_hydration_records(
    home: &DevRelayHome,
    project_id: &str,
) -> Result<Vec<HydrationStateRecord>> {
    let dir = home.project_data_dir(project_id).join("hydration");
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let mut records = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
            records.push(crate::load_hydration_state(&path)?);
        }
    }
    records.sort_by(|left, right| {
        left.project_id
            .cmp(&right.project_id)
            .then(left.workspace_id.cmp(&right.workspace_id))
    });
    Ok(records)
}

fn continuation_metrics(handoffs: &[LocalMetricsHandoffInput]) -> VerifiedContinuationMetrics {
    let verified_attempts = handoffs
        .iter()
        .filter(|handoff| {
            handoff.record.state == HandoffState::TargetVerified
                || handoff.record.state == HandoffState::SourceReady
                || handoff.record.state == HandoffState::Committed
                || handoff
                    .journal
                    .iter()
                    .any(|entry| entry.phase == HandoffJournalPhase::TargetVerified)
        })
        .count();
    let successes = handoffs
        .iter()
        .filter(|handoff| {
            handoff.record.state == HandoffState::Committed
                || handoff
                    .journal
                    .iter()
                    .any(|entry| entry.phase == HandoffJournalPhase::LeaseCommitted)
        })
        .count();
    let aborted = handoffs
        .iter()
        .filter(|handoff| handoff.record.state == HandoffState::Aborted)
        .count();
    VerifiedContinuationMetrics {
        handoff_attempts: handoffs.len(),
        verified_attempts,
        successes,
        aborted,
    }
}

fn checkpoint_metrics(
    snapshots: &[StoredSnapshot],
    audits: &[AuditEventRecord],
) -> CheckpointMetrics {
    let failure_reasons = reason_counts(audits.iter().filter_map(|event| {
        (event.event_type == AuditEventType::SnapshotPublished
            && event.outcome != AuditOutcome::Succeeded)
            .then(|| event.summary.clone())
    }));
    CheckpointMetrics {
        successes: snapshots.len(),
        failure_reasons,
    }
}

fn apply_metrics(audits: &[AuditEventRecord]) -> ApplyVerificationMetrics {
    let failures = audits
        .iter()
        .filter(|event| {
            event.event_type == AuditEventType::SnapshotApplied
                && event.outcome != AuditOutcome::Succeeded
        })
        .collect::<Vec<_>>();
    ApplyVerificationMetrics {
        successful_applies: audits
            .iter()
            .filter(|event| {
                event.event_type == AuditEventType::SnapshotApplied
                    && event.outcome == AuditOutcome::Succeeded
            })
            .count(),
        verification_failures: failures.len(),
        verification_failure_reasons: reason_counts(failures.into_iter().map(|event| {
            if event.summary.is_empty() {
                "snapshot apply verification failed".to_string()
            } else {
                event.summary.clone()
            }
        })),
    }
}

fn handoff_metrics(handoffs: &[LocalMetricsHandoffInput]) -> HandoffMetrics {
    let mut phase_durations = Vec::new();
    let mut committed_total_durations = Vec::new();
    for handoff in handoffs {
        let mut journal = handoff.journal.clone();
        journal.sort_by(|left, right| {
            left.journal_id.cmp(&right.journal_id).then(
                left.created_at_unix_seconds
                    .cmp(&right.created_at_unix_seconds),
            )
        });
        for pair in journal.windows(2) {
            let from = &pair[0];
            let to = &pair[1];
            phase_durations.push(HandoffPhaseDurationMetric {
                project_id: handoff.record.project_id.clone(),
                handoff_id: handoff.record.handoff_id.clone(),
                from_phase: from.phase,
                to_phase: to.phase,
                duration_seconds: to
                    .created_at_unix_seconds
                    .saturating_sub(from.created_at_unix_seconds),
            });
        }
        let begin = journal
            .iter()
            .find(|entry| entry.phase == HandoffJournalPhase::Begin);
        let committed = journal
            .iter()
            .find(|entry| entry.phase == HandoffJournalPhase::LeaseCommitted);
        if let (Some(begin), Some(committed)) = (begin, committed) {
            committed_total_durations.push(HandoffTotalDurationMetric {
                project_id: handoff.record.project_id.clone(),
                handoff_id: handoff.record.handoff_id.clone(),
                duration_seconds: committed
                    .created_at_unix_seconds
                    .saturating_sub(begin.created_at_unix_seconds),
            });
        }
    }
    HandoffMetrics {
        phase_durations,
        committed_total_durations,
    }
}

fn environment_metrics(records: &[HydrationStateRecord]) -> EnvironmentHydrationMetrics {
    let duration_samples = records
        .iter()
        .filter_map(|record| {
            let started = record.started_at_unix_seconds?;
            let ready = record.ready_at_unix_seconds?;
            Some(EnvironmentHydrationDurationMetric {
                project_id: record.project_id.clone(),
                workspace_id: record.workspace_id.clone(),
                duration_seconds: ready.saturating_sub(started),
            })
        })
        .collect();
    EnvironmentHydrationMetrics {
        records: records.len(),
        shell_ready: records
            .iter()
            .filter(|record| record.state == HydrationState::ShellReady)
            .count(),
        app_ready: records
            .iter()
            .filter(|record| record.state == HydrationState::AppReady)
            .count(),
        failed: records
            .iter()
            .filter(|record| record.state == HydrationState::Failed)
            .count(),
        duration_samples,
    }
}

fn scheduler_metrics(task_runs: &[TaskRunRecord]) -> SchedulerChoiceMetrics {
    let mut missing = 0;
    let mut counts = BTreeMap::<(String, Option<String>), usize>::new();
    for task_run in task_runs {
        if let Some(choice) = scheduler_choice_reason(&task_run.metadata) {
            *counts.entry(choice).or_default() += 1;
        } else {
            missing += 1;
        }
    }
    let mut choice_reasons = counts
        .into_iter()
        .map(
            |((reason, explanation), count)| SchedulerChoiceReasonCount {
                reason,
                explanation,
                count,
            },
        )
        .collect::<Vec<_>>();
    choice_reasons.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then(left.reason.cmp(&right.reason))
            .then(left.explanation.cmp(&right.explanation))
    });
    SchedulerChoiceMetrics {
        task_runs_with_choice_reason: task_runs.len().saturating_sub(missing),
        task_runs_missing_choice_reason: missing,
        choice_reasons,
    }
}

fn scheduler_choice_reason(metadata: &Value) -> Option<(String, Option<String>)> {
    let direct_reason = metadata
        .get("scheduler_choice_reason")
        .or_else(|| metadata.get("scheduler_reason"))
        .or_else(|| metadata.pointer("/scheduler/choice_reason"))
        .or_else(|| metadata.pointer("/scheduler/reason"))
        .and_then(Value::as_str)
        .map(ToString::to_string);
    let explanation = metadata
        .get("scheduler_explanation")
        .or_else(|| metadata.pointer("/scheduler/explanation"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            metadata
                .pointer("/scheduler_selection/explanation")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" | ")
                })
                .filter(|value| !value.is_empty())
        });
    if let Some(reason) = direct_reason {
        return Some((reason, explanation));
    }
    metadata
        .pointer("/scheduler_selection/selected_device_id")
        .and_then(Value::as_str)
        .map(|device_id| (format!("selected {device_id}"), explanation))
}

fn reason_counts(reasons: impl IntoIterator<Item = String>) -> Vec<MetricReasonCount> {
    let mut counts = BTreeMap::<String, usize>::new();
    for reason in reasons {
        *counts.entry(reason).or_default() += 1;
    }
    let mut reasons = counts
        .into_iter()
        .map(|(reason, count)| MetricReasonCount { reason, count })
        .collect::<Vec<_>>();
    reasons.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then(left.reason.cmp(&right.reason))
    });
    reasons
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SnapshotMetadata, StatusCounts, TaskRunState};

    #[test]
    fn aggregates_local_metrics_and_redacts_reason_text() {
        let report = build_local_metrics_report(LocalMetricsInput {
            generated_at_unix_seconds: 500,
            project: Some("project-a".to_string()),
            redacted: false,
            audits: vec![
                audit(
                    AuditEventType::SnapshotPublished,
                    AuditOutcome::Blocked,
                    "checkpoint failed in /Users/me/project token=secret-value",
                ),
                audit(
                    AuditEventType::SnapshotApplied,
                    AuditOutcome::Failed,
                    "apply verification failed at /Users/me/project",
                ),
                audit(
                    AuditEventType::SnapshotApplied,
                    AuditOutcome::Succeeded,
                    "snapshot applied",
                ),
            ],
            snapshots: vec![snapshot("s1"), snapshot("s2")],
            handoffs: vec![handoff()],
            task_runs: vec![
                TaskRunRecord {
                    task_run_id: "tr_1".to_string(),
                    project_id: "project-a".to_string(),
                    session_id: None,
                    state: TaskRunState::Succeeded.as_str().to_string(),
                    command: Some("cargo test".to_string()),
                    metadata: serde_json::json!({
                        "scheduler_reason": "cache-warm",
                        "scheduler_explanation": "selected /Users/me/project runner",
                    }),
                    created_at_unix_seconds: 10,
                    updated_at_unix_seconds: 20,
                },
                TaskRunRecord {
                    task_run_id: "tr_2".to_string(),
                    project_id: "project-a".to_string(),
                    session_id: None,
                    state: TaskRunState::Queued.as_str().to_string(),
                    command: None,
                    metadata: serde_json::json!({}),
                    created_at_unix_seconds: 10,
                    updated_at_unix_seconds: 20,
                },
            ],
            hydration: vec![
                hydration(HydrationState::ShellReady),
                hydration(HydrationState::Failed),
            ],
        });

        assert_eq!(report.privacy.local_by_default, true);
        assert_eq!(report.privacy.source_code_included, false);
        assert_eq!(report.continuation.verified_attempts, 1);
        assert_eq!(report.continuation.successes, 1);
        assert_eq!(report.checkpoints.successes, 2);
        assert_eq!(report.checkpoints.failure_reasons[0].count, 1);
        assert_eq!(report.apply.successful_applies, 1);
        assert_eq!(report.apply.verification_failures, 1);
        assert_eq!(report.handoffs.phase_durations.len(), 2);
        assert_eq!(
            report.handoffs.committed_total_durations[0].duration_seconds,
            8
        );
        assert_eq!(report.environment.shell_ready, 1);
        assert_eq!(report.environment.failed, 1);
        assert_eq!(report.environment.duration_samples[0].duration_seconds, 12);
        assert_eq!(report.scheduler.task_runs_with_choice_reason, 1);
        assert_eq!(report.scheduler.task_runs_missing_choice_reason, 1);
        assert!(report.recording_gaps.is_empty());

        let redactor =
            LogRedactor::for_diagnostics([std::path::PathBuf::from("/Users/me/project")]);
        let redacted = report.redact(&redactor);
        let encoded = serde_json::to_string(&redacted).unwrap();
        assert!(!encoded.contains("/Users/me/project"));
        assert!(!encoded.contains("secret-value"));
        assert!(encoded.contains("<path>"));
        assert!(encoded.contains("<redacted>"));
    }

    fn audit(event_type: AuditEventType, outcome: AuditOutcome, summary: &str) -> AuditEventRecord {
        AuditEventRecord {
            schema_version: crate::AUDIT_SCHEMA_VERSION,
            audit_id: 1,
            event_type,
            outcome,
            summary: summary.to_string(),
            project_id: Some("project-a".to_string()),
            actor_device_id: None,
            target_device_id: None,
            session_id: None,
            snapshot_id: None,
            lease_id: None,
            handoff_id: None,
            detail: serde_json::json!({}),
            created_at_unix_seconds: 100,
        }
    }

    fn snapshot(snapshot_id: &str) -> StoredSnapshot {
        let snapshot_id = format!("s1_{snapshot_id:0<24}");
        StoredSnapshot {
            snapshot_id: snapshot_id.clone(),
            project_id: "project-a".to_string(),
            session_id: None,
            parent_snapshot_id: None,
            sequence_number: 1,
            pinned: false,
            label: None,
            metadata: SnapshotMetadata {
                schema_version: 1,
                snapshot_id,
                project_id: "project-a".to_string(),
                project_name: "Project A".to_string(),
                session_id: None,
                parent_snapshot_id: None,
                child_snapshots: Vec::new(),
                source_device_id: None,
                branch: Some("main".to_string()),
                head_oid: "a".repeat(40),
                index_tree_oid: "b".repeat(40),
                index_commit_oid: "c".repeat(40),
                work_tree_oid: "d".repeat(40),
                work_commit_oid: "e".repeat(40),
                source_status: StatusCounts::default(),
                operation_capsule: None,
                included_untracked: Vec::new(),
                excluded: Vec::new(),
                sidecars: Vec::new(),
                state_hash: "state-hash".to_string(),
                created_at_unix_seconds: 100,
            },
            created_at_unix_seconds: 100,
        }
    }

    fn handoff() -> LocalMetricsHandoffInput {
        let record = HandoffRecord {
            handoff_id: "ho_1".to_string(),
            lease_id: "lease-1".to_string(),
            project_id: "project-a".to_string(),
            expected_epoch: 1,
            source_device_id: "source".to_string(),
            target_device_id: "target".to_string(),
            source_generation: "gen-1".to_string(),
            expires_at_unix_seconds: 1_000,
            state: HandoffState::Committed,
        };
        let journal = vec![
            journal(1, HandoffJournalPhase::Begin, 10),
            journal(2, HandoffJournalPhase::TargetVerified, 13),
            journal(3, HandoffJournalPhase::LeaseCommitted, 18),
        ];
        LocalMetricsHandoffInput { record, journal }
    }

    fn journal(
        journal_id: i64,
        phase: HandoffJournalPhase,
        created_at_unix_seconds: u64,
    ) -> HandoffJournalRecord {
        HandoffJournalRecord {
            journal_id,
            handoff_id: "ho_1".to_string(),
            lease_id: "lease-1".to_string(),
            project_id: "project-a".to_string(),
            phase,
            detail_json: "{}".to_string(),
            created_at_unix_seconds,
        }
    }

    fn hydration(state: HydrationState) -> HydrationStateRecord {
        let mut record = HydrationStateRecord::new("project-a", None, 100);
        record.state = state;
        if matches!(state, HydrationState::ShellReady | HydrationState::AppReady) {
            record.ready_at_unix_seconds = Some(112);
        }
        record
    }
}
