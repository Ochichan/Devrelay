//! Retention and quota planning for local and anchor snapshot caches.
//!
//! The planner is intentionally separate from deletion. It produces a stable
//! list of snapshots that may be pruned while preserving hard safety rules for
//! canonical latest, pinned, and active handoff-protected snapshots.

use crate::{StoredSnapshot, SyncConfig};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MIB: u64 = 1024 * 1024;
const SECONDS_PER_HOUR: u64 = 60 * 60;
const SECONDS_PER_DAY: u64 = 24 * SECONDS_PER_HOUR;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PruningScope {
    DeviceCache,
    AnchorProject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub hot_snapshot_count: usize,
    pub hourly_thinning_hours: u64,
    pub daily_thinning_days: u64,
    pub handoff_protection_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_cache_quota_mib: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor_project_quota_mib: Option<u64>,
    pub free_disk_warning_threshold_mib: u64,
    pub free_disk_hard_stop_threshold_mib: u64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            hot_snapshot_count: 5,
            hourly_thinning_hours: 24,
            daily_thinning_days: 14,
            handoff_protection_seconds: SECONDS_PER_DAY,
            device_cache_quota_mib: None,
            anchor_project_quota_mib: None,
            free_disk_warning_threshold_mib: 1024,
            free_disk_hard_stop_threshold_mib: 256,
        }
    }
}

impl RetentionPolicy {
    pub fn from_sync_config(sync: Option<&SyncConfig>) -> Self {
        let mut policy = Self::default();
        if let Some(sync) = sync {
            policy.device_cache_quota_mib = sync.device_cache_quota_mib;
        }
        policy
    }

    pub fn quota_bytes(self, scope: PruningScope) -> Option<u64> {
        match scope {
            PruningScope::DeviceCache => self.device_cache_quota_mib.map(mib_to_bytes),
            PruningScope::AnchorProject => self.anchor_project_quota_mib.map(mib_to_bytes),
        }
    }

    pub fn free_disk_warning_threshold_bytes(self) -> u64 {
        mib_to_bytes(self.free_disk_warning_threshold_mib)
    }

    pub fn free_disk_hard_stop_threshold_bytes(self) -> u64 {
        mib_to_bytes(self.free_disk_hard_stop_threshold_mib)
    }

    pub fn handoff_protection(
        self,
        snapshot_id: impl Into<String>,
        protected_at_unix_seconds: u64,
    ) -> HandoffSnapshotProtection {
        HandoffSnapshotProtection {
            snapshot_id: snapshot_id.into(),
            protect_until_unix_seconds: protected_at_unix_seconds
                .saturating_add(self.handoff_protection_seconds),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRetentionEntry {
    pub snapshot_id: String,
    pub sequence_number: i64,
    pub pinned: bool,
    pub created_at_unix_seconds: u64,
    pub estimated_size_bytes: u64,
}

impl SnapshotRetentionEntry {
    pub fn from_stored(snapshot: &StoredSnapshot, estimated_size_bytes: u64) -> Self {
        Self {
            snapshot_id: snapshot.snapshot_id.clone(),
            sequence_number: snapshot.sequence_number,
            pinned: snapshot.pinned,
            created_at_unix_seconds: snapshot.created_at_unix_seconds,
            estimated_size_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffSnapshotProtection {
    pub snapshot_id: String,
    pub protect_until_unix_seconds: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RetentionKeepReason {
    CanonicalLatest,
    Pinned,
    HandoffProtected,
    HotSnapshot,
    HourlyThinning,
    DailyThinning,
}

impl RetentionKeepReason {
    fn is_hard(self) -> bool {
        matches!(
            self,
            Self::CanonicalLatest | Self::Pinned | Self::HandoffProtected
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PruningReason {
    OutsideRetention,
    QuotaPressure,
    FreeDiskPressure,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum PruningDecisionAction {
    Keep { reasons: Vec<RetentionKeepReason> },
    Delete { reason: PruningReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruningDecision {
    pub snapshot_id: String,
    pub estimated_size_bytes: u64,
    pub action: PruningDecisionAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PruningPlanWarningCode {
    QuotaExceeded,
    FreeDiskLow,
    FreeDiskHardStop,
    InsufficientReclaimableSnapshots,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruningPlanWarning {
    pub code: PruningPlanWarningCode,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PruningPlan {
    pub scope: PruningScope,
    pub current_usage_bytes: u64,
    pub free_disk_bytes: u64,
    pub target_reclaim_bytes: u64,
    pub planned_reclaim_bytes: u64,
    pub decisions: Vec<PruningDecision>,
    pub warnings: Vec<PruningPlanWarning>,
}

impl PruningPlan {
    pub fn delete_snapshot_ids(&self) -> Vec<String> {
        self.decisions
            .iter()
            .filter_map(|decision| match decision.action {
                PruningDecisionAction::Delete { .. } => Some(decision.snapshot_id.clone()),
                PruningDecisionAction::Keep { .. } => None,
            })
            .collect()
    }

    pub fn protected_snapshot_ids(&self) -> Vec<String> {
        self.decisions
            .iter()
            .filter_map(|decision| match &decision.action {
                PruningDecisionAction::Keep { reasons }
                    if reasons.iter().any(|reason| reason.is_hard()) =>
                {
                    Some(decision.snapshot_id.clone())
                }
                _ => None,
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PruningPlanInput<'a> {
    pub snapshots: &'a [SnapshotRetentionEntry],
    pub policy: RetentionPolicy,
    pub scope: PruningScope,
    pub canonical_latest_snapshot_id: Option<&'a str>,
    pub handoff_protections: &'a [HandoffSnapshotProtection],
    pub current_usage_bytes: u64,
    pub free_disk_bytes: u64,
    pub now_unix_seconds: u64,
}

pub fn plan_snapshot_pruning(input: PruningPlanInput<'_>) -> PruningPlan {
    let PruningPlanInput {
        snapshots,
        policy,
        scope,
        canonical_latest_snapshot_id,
        handoff_protections,
        current_usage_bytes,
        free_disk_bytes,
        now_unix_seconds,
    } = input;
    let mut reasons = retention_reasons(
        snapshots,
        policy,
        canonical_latest_snapshot_id,
        handoff_protections,
        now_unix_seconds,
    );
    let quota_overage = policy
        .quota_bytes(scope)
        .and_then(|quota| current_usage_bytes.checked_sub(quota))
        .unwrap_or_default();
    let hard_stop_deficit = policy
        .free_disk_hard_stop_threshold_bytes()
        .saturating_sub(free_disk_bytes);
    let target_reclaim_bytes = quota_overage.max(hard_stop_deficit);

    let mut delete_reasons = BTreeMap::new();
    for snapshot in snapshots {
        if reasons
            .get(&snapshot.snapshot_id)
            .is_none_or(BTreeSet::is_empty)
        {
            delete_reasons.insert(
                snapshot.snapshot_id.clone(),
                PruningReason::OutsideRetention,
            );
        }
    }

    let mut planned_reclaim_bytes =
        planned_reclaim_bytes(snapshots, delete_reasons.keys().map(String::as_str));
    if planned_reclaim_bytes < target_reclaim_bytes {
        let pressure_reason = if quota_overage >= hard_stop_deficit && quota_overage > 0 {
            PruningReason::QuotaPressure
        } else {
            PruningReason::FreeDiskPressure
        };
        for snapshot in oldest_first(snapshots) {
            if planned_reclaim_bytes >= target_reclaim_bytes {
                break;
            }
            if delete_reasons.contains_key(&snapshot.snapshot_id) {
                continue;
            }
            let snapshot_reasons = reasons.entry(snapshot.snapshot_id.clone()).or_default();
            if snapshot_reasons.iter().any(|reason| reason.is_hard()) {
                continue;
            }
            snapshot_reasons.clear();
            delete_reasons.insert(snapshot.snapshot_id.clone(), pressure_reason);
            planned_reclaim_bytes =
                planned_reclaim_bytes.saturating_add(snapshot.estimated_size_bytes);
        }
    }

    let decisions = snapshots
        .iter()
        .map(|snapshot| {
            if let Some(reason) = delete_reasons.get(&snapshot.snapshot_id) {
                PruningDecision {
                    snapshot_id: snapshot.snapshot_id.clone(),
                    estimated_size_bytes: snapshot.estimated_size_bytes,
                    action: PruningDecisionAction::Delete { reason: *reason },
                }
            } else {
                let mut reasons = reasons
                    .get(&snapshot.snapshot_id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .collect::<Vec<_>>();
                reasons.sort();
                PruningDecision {
                    snapshot_id: snapshot.snapshot_id.clone(),
                    estimated_size_bytes: snapshot.estimated_size_bytes,
                    action: PruningDecisionAction::Keep { reasons },
                }
            }
        })
        .collect();

    let mut warnings = Vec::new();
    if quota_overage > 0 {
        warnings.push(PruningPlanWarning {
            code: PruningPlanWarningCode::QuotaExceeded,
            message: format!("snapshot cache exceeds configured quota by {quota_overage} bytes"),
        });
    }
    if free_disk_bytes < policy.free_disk_warning_threshold_bytes() {
        warnings.push(PruningPlanWarning {
            code: PruningPlanWarningCode::FreeDiskLow,
            message: "free disk is below warning threshold".to_string(),
        });
    }
    if hard_stop_deficit > 0 {
        warnings.push(PruningPlanWarning {
            code: PruningPlanWarningCode::FreeDiskHardStop,
            message: "free disk is below hard stop threshold".to_string(),
        });
    }
    if planned_reclaim_bytes < target_reclaim_bytes {
        warnings.push(PruningPlanWarning {
            code: PruningPlanWarningCode::InsufficientReclaimableSnapshots,
            message: format!(
                "planner can reclaim {planned_reclaim_bytes} bytes but target is {target_reclaim_bytes} bytes"
            ),
        });
    }

    PruningPlan {
        scope,
        current_usage_bytes,
        free_disk_bytes,
        target_reclaim_bytes,
        planned_reclaim_bytes,
        decisions,
        warnings,
    }
}

fn retention_reasons(
    snapshots: &[SnapshotRetentionEntry],
    policy: RetentionPolicy,
    canonical_latest_snapshot_id: Option<&str>,
    handoff_protections: &[HandoffSnapshotProtection],
    now_unix_seconds: u64,
) -> BTreeMap<String, BTreeSet<RetentionKeepReason>> {
    let mut reasons = BTreeMap::<String, BTreeSet<RetentionKeepReason>>::new();

    if let Some(snapshot_id) = canonical_latest_snapshot_id {
        add_reason(
            &mut reasons,
            snapshot_id,
            RetentionKeepReason::CanonicalLatest,
        );
    }
    for snapshot in snapshots {
        if snapshot.pinned {
            add_reason(
                &mut reasons,
                &snapshot.snapshot_id,
                RetentionKeepReason::Pinned,
            );
        }
    }
    for protection in handoff_protections {
        if protection.protect_until_unix_seconds >= now_unix_seconds {
            add_reason(
                &mut reasons,
                &protection.snapshot_id,
                RetentionKeepReason::HandoffProtected,
            );
        }
    }

    for snapshot in newest_first(snapshots)
        .into_iter()
        .take(policy.hot_snapshot_count)
    {
        add_reason(
            &mut reasons,
            &snapshot.snapshot_id,
            RetentionKeepReason::HotSnapshot,
        );
    }

    let hourly_floor =
        now_unix_seconds.saturating_sub(policy.hourly_thinning_hours * SECONDS_PER_HOUR);
    for snapshot in latest_per_bucket(snapshots, hourly_floor, SECONDS_PER_HOUR).values() {
        add_reason(
            &mut reasons,
            &snapshot.snapshot_id,
            RetentionKeepReason::HourlyThinning,
        );
    }

    let daily_floor = now_unix_seconds.saturating_sub(policy.daily_thinning_days * SECONDS_PER_DAY);
    for snapshot in latest_per_bucket(snapshots, daily_floor, SECONDS_PER_DAY).values() {
        add_reason(
            &mut reasons,
            &snapshot.snapshot_id,
            RetentionKeepReason::DailyThinning,
        );
    }

    reasons
}

fn add_reason(
    reasons: &mut BTreeMap<String, BTreeSet<RetentionKeepReason>>,
    snapshot_id: impl AsRef<str>,
    reason: RetentionKeepReason,
) {
    reasons
        .entry(snapshot_id.as_ref().to_string())
        .or_default()
        .insert(reason);
}

fn newest_first(snapshots: &[SnapshotRetentionEntry]) -> Vec<&SnapshotRetentionEntry> {
    let mut snapshots = snapshots.iter().collect::<Vec<_>>();
    snapshots.sort_by(|left, right| {
        right
            .sequence_number
            .cmp(&left.sequence_number)
            .then(
                right
                    .created_at_unix_seconds
                    .cmp(&left.created_at_unix_seconds),
            )
            .then(left.snapshot_id.cmp(&right.snapshot_id))
    });
    snapshots
}

fn oldest_first(snapshots: &[SnapshotRetentionEntry]) -> Vec<&SnapshotRetentionEntry> {
    let mut snapshots = snapshots.iter().collect::<Vec<_>>();
    snapshots.sort_by(|left, right| {
        left.sequence_number
            .cmp(&right.sequence_number)
            .then(
                left.created_at_unix_seconds
                    .cmp(&right.created_at_unix_seconds),
            )
            .then(left.snapshot_id.cmp(&right.snapshot_id))
    });
    snapshots
}

fn latest_per_bucket(
    snapshots: &[SnapshotRetentionEntry],
    floor_unix_seconds: u64,
    bucket_seconds: u64,
) -> BTreeMap<u64, &SnapshotRetentionEntry> {
    let mut buckets = BTreeMap::new();
    for snapshot in snapshots {
        if snapshot.created_at_unix_seconds < floor_unix_seconds {
            continue;
        }
        let bucket = snapshot.created_at_unix_seconds / bucket_seconds;
        let replace = buckets
            .get(&bucket)
            .map(|current: &&SnapshotRetentionEntry| {
                snapshot.sequence_number > current.sequence_number
                    || (snapshot.sequence_number == current.sequence_number
                        && snapshot.created_at_unix_seconds > current.created_at_unix_seconds)
            })
            .unwrap_or(true);
        if replace {
            buckets.insert(bucket, snapshot);
        }
    }
    buckets
}

fn planned_reclaim_bytes<'a>(
    snapshots: &[SnapshotRetentionEntry],
    delete_ids: impl Iterator<Item = &'a str>,
) -> u64 {
    let delete_ids = delete_ids.collect::<BTreeSet<_>>();
    snapshots
        .iter()
        .filter(|snapshot| delete_ids.contains(snapshot.snapshot_id.as_str()))
        .map(|snapshot| snapshot.estimated_size_bytes)
        .sum()
}

fn mib_to_bytes(value: u64) -> u64 {
    value.saturating_mul(MIB)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(
        snapshot_id: &str,
        sequence_number: i64,
        created_at_unix_seconds: u64,
        pinned: bool,
    ) -> SnapshotRetentionEntry {
        SnapshotRetentionEntry {
            snapshot_id: snapshot_id.to_string(),
            sequence_number,
            pinned,
            created_at_unix_seconds,
            estimated_size_bytes: 100,
        }
    }

    fn compact_policy() -> RetentionPolicy {
        RetentionPolicy {
            hot_snapshot_count: 2,
            hourly_thinning_hours: 3,
            daily_thinning_days: 2,
            handoff_protection_seconds: SECONDS_PER_DAY,
            device_cache_quota_mib: None,
            anchor_project_quota_mib: None,
            free_disk_warning_threshold_mib: 100,
            free_disk_hard_stop_threshold_mib: 10,
        }
    }

    fn keep_reasons(plan: &PruningPlan, snapshot_id: &str) -> Vec<RetentionKeepReason> {
        plan.decisions
            .iter()
            .find(|decision| decision.snapshot_id == snapshot_id)
            .and_then(|decision| match &decision.action {
                PruningDecisionAction::Keep { reasons } => Some(reasons.clone()),
                PruningDecisionAction::Delete { .. } => None,
            })
            .unwrap_or_default()
    }

    #[test]
    fn retention_keeps_hot_hourly_daily_latest_pinned_and_handoff_snapshots() {
        let now = 10 * SECONDS_PER_DAY;
        let snapshots = vec![
            snapshot("s_old_delete", 1, now - 10 * SECONDS_PER_DAY, false),
            snapshot("s_daily_keep", 2, now - 30 * SECONDS_PER_HOUR, false),
            snapshot("s_hourly_keep", 3, now - 2 * SECONDS_PER_HOUR, false),
            snapshot("s_latest", 4, now - SECONDS_PER_HOUR, false),
            snapshot("s_pinned", 5, now - 30 * 60, true),
            snapshot("s_handoff", 6, now - 15 * 60, false),
        ];

        let plan = plan_snapshot_pruning(PruningPlanInput {
            snapshots: &snapshots,
            policy: compact_policy(),
            scope: PruningScope::DeviceCache,
            canonical_latest_snapshot_id: Some("s_latest"),
            handoff_protections: &[compact_policy().handoff_protection("s_handoff", now)],
            current_usage_bytes: 600,
            free_disk_bytes: 1_000 * MIB,
            now_unix_seconds: now,
        });

        assert_eq!(plan.delete_snapshot_ids(), vec!["s_old_delete".to_string()]);
        assert!(keep_reasons(&plan, "s_latest").contains(&RetentionKeepReason::CanonicalLatest));
        assert!(keep_reasons(&plan, "s_pinned").contains(&RetentionKeepReason::Pinned));
        assert!(keep_reasons(&plan, "s_handoff").contains(&RetentionKeepReason::HandoffProtected));
        assert!(keep_reasons(&plan, "s_handoff").contains(&RetentionKeepReason::HotSnapshot));
        assert!(
            keep_reasons(&plan, "s_hourly_keep").contains(&RetentionKeepReason::HourlyThinning)
        );
        assert!(keep_reasons(&plan, "s_daily_keep").contains(&RetentionKeepReason::DailyThinning));
        assert_eq!(
            plan.protected_snapshot_ids(),
            vec![
                "s_latest".to_string(),
                "s_pinned".to_string(),
                "s_handoff".to_string()
            ]
        );
    }

    #[test]
    fn quota_pressure_prunes_soft_retained_snapshots_but_not_hard_protected_snapshots() {
        let now = 1_000_000;
        let mut policy = compact_policy();
        policy.hot_snapshot_count = 3;
        policy.device_cache_quota_mib = Some(1);
        let snapshots = vec![
            SnapshotRetentionEntry {
                estimated_size_bytes: 900 * 1024,
                ..snapshot("s_old_hot", 1, now - 100, false)
            },
            SnapshotRetentionEntry {
                estimated_size_bytes: 900 * 1024,
                ..snapshot("s_pinned", 2, now - 50, true)
            },
            SnapshotRetentionEntry {
                estimated_size_bytes: 900 * 1024,
                ..snapshot("s_latest", 3, now, false)
            },
        ];

        let plan = plan_snapshot_pruning(PruningPlanInput {
            snapshots: &snapshots,
            policy,
            scope: PruningScope::DeviceCache,
            canonical_latest_snapshot_id: Some("s_latest"),
            handoff_protections: &[],
            current_usage_bytes: 2700 * 1024,
            free_disk_bytes: 1_000 * MIB,
            now_unix_seconds: now,
        });

        assert_eq!(plan.delete_snapshot_ids(), vec!["s_old_hot".to_string()]);
        assert!(keep_reasons(&plan, "s_pinned").contains(&RetentionKeepReason::Pinned));
        assert!(keep_reasons(&plan, "s_latest").contains(&RetentionKeepReason::CanonicalLatest));
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.code == PruningPlanWarningCode::QuotaExceeded)
        );
    }

    #[test]
    fn anchor_project_quota_uses_anchor_budget() {
        let now = 1_000_000;
        let mut policy = compact_policy();
        policy.hot_snapshot_count = 0;
        policy.device_cache_quota_mib = Some(100);
        policy.anchor_project_quota_mib = Some(1);
        let snapshots = vec![
            SnapshotRetentionEntry {
                estimated_size_bytes: 800 * 1024,
                ..snapshot("s1", 1, now - 100, false)
            },
            SnapshotRetentionEntry {
                estimated_size_bytes: 800 * 1024,
                ..snapshot("s2", 2, now - 50, false)
            },
        ];

        let device_plan = plan_snapshot_pruning(PruningPlanInput {
            snapshots: &snapshots,
            policy,
            scope: PruningScope::DeviceCache,
            canonical_latest_snapshot_id: None,
            handoff_protections: &[],
            current_usage_bytes: 1600 * 1024,
            free_disk_bytes: 1_000 * MIB,
            now_unix_seconds: now,
        });
        let anchor_plan = plan_snapshot_pruning(PruningPlanInput {
            snapshots: &snapshots,
            policy,
            scope: PruningScope::AnchorProject,
            canonical_latest_snapshot_id: None,
            handoff_protections: &[],
            current_usage_bytes: 1600 * 1024,
            free_disk_bytes: 1_000 * MIB,
            now_unix_seconds: now,
        });

        assert!(
            !device_plan
                .warnings
                .iter()
                .any(|warning| warning.code == PruningPlanWarningCode::QuotaExceeded)
        );
        assert!(
            anchor_plan
                .warnings
                .iter()
                .any(|warning| warning.code == PruningPlanWarningCode::QuotaExceeded)
        );
    }

    #[test]
    fn free_disk_thresholds_warn_and_drive_reclaim_target() {
        let now = 1_000_000;
        let policy = compact_policy();
        let snapshots = vec![
            snapshot("s_reclaim", 1, now - 10 * SECONDS_PER_DAY, false),
            snapshot("s_latest", 2, now, false),
        ];

        let plan = plan_snapshot_pruning(PruningPlanInput {
            snapshots: &snapshots,
            policy,
            scope: PruningScope::DeviceCache,
            canonical_latest_snapshot_id: Some("s_latest"),
            handoff_protections: &[],
            current_usage_bytes: 200,
            free_disk_bytes: 5 * MIB,
            now_unix_seconds: now,
        });

        assert_eq!(plan.target_reclaim_bytes, 5 * MIB);
        assert!(
            plan.delete_snapshot_ids()
                .contains(&"s_reclaim".to_string())
        );
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.code == PruningPlanWarningCode::FreeDiskLow)
        );
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.code == PruningPlanWarningCode::FreeDiskHardStop)
        );
        assert!(plan.warnings.iter().any(
            |warning| warning.code == PruningPlanWarningCode::InsufficientReclaimableSnapshots
        ));
    }

    #[test]
    fn sync_config_supplies_device_cache_quota() {
        let policy = RetentionPolicy::from_sync_config(Some(&SyncConfig {
            mode: None,
            checkpoint_quiet_ms: None,
            publish_quiet_ms: None,
            max_publish_interval_s: None,
            background_bandwidth_mib_s: None,
            device_cache_quota_mib: Some(42),
        }));

        assert_eq!(
            policy.quota_bytes(PruningScope::DeviceCache),
            Some(42 * MIB)
        );
    }
}
