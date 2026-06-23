//! Event stream envelope types shared by the agent and local clients.
//!
//! M2 events are ordered by a single local agent sequence. The sequence is the
//! replay cursor boundary; timestamps are informational and must not be used to
//! infer ordering.

use crate::{Result, StoredSnapshot, VerificationDetails, WorkspaceState};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const EVENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventSequence(u64);

impl EventSequence {
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Self::new(value),
            None => None,
        }
    }
}

impl Serialize for EventSequence {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(self.0)
    }
}

impl<'de> Deserialize<'de> for EventSequence {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u64::deserialize(deserializer)?;
        Self::new(value).ok_or_else(|| de::Error::custom("event sequence must be greater than 0"))
    }
}

#[derive(Debug)]
pub struct EventSequencer {
    next_sequence: AtomicU64,
}

impl EventSequencer {
    pub const fn new() -> Self {
        Self {
            next_sequence: AtomicU64::new(1),
        }
    }

    pub fn starting_after(sequence: EventSequence) -> Option<Self> {
        sequence.next().map(Self::starting_at)
    }

    pub fn starting_at(sequence: EventSequence) -> Self {
        Self {
            next_sequence: AtomicU64::new(sequence.get()),
        }
    }

    pub fn next(&self) -> Option<EventSequence> {
        let value = self
            .next_sequence
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                current.checked_add(1)
            })
            .ok()?;
        EventSequence::new(value)
    }
}

impl Default for EventSequencer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventTimestampMillis(u64);

impl EventTimestampMillis {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn now() -> Self {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        Self(millis)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventReplayCursor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_sequence: Option<EventSequence>,
}

impl EventReplayCursor {
    pub const fn from_start() -> Self {
        Self {
            after_sequence: None,
        }
    }

    pub const fn after(sequence: EventSequence) -> Self {
        Self {
            after_sequence: Some(sequence),
        }
    }

    pub fn accepts(self, sequence: EventSequence) -> bool {
        match self.after_sequence {
            Some(after_sequence) => sequence > after_sequence,
            None => true,
        }
    }
}

impl Default for EventReplayCursor {
    fn default() -> Self {
        Self::from_start()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    #[serde(rename = "workspace.state.changed")]
    WorkspaceStateChanged,
    #[serde(rename = "snapshot.local.created")]
    SnapshotLocalCreated,
    #[serde(rename = "snapshot.apply.started")]
    SnapshotApplyStarted,
    #[serde(rename = "snapshot.apply.verified")]
    SnapshotApplyVerified,
    #[serde(rename = "security.blocked")]
    SecurityBlocked,
    #[serde(rename = "quota.warning")]
    QuotaWarning,
}

pub trait TypedEventPayload: Serialize {
    fn event_type(&self) -> EventType;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceStateChangedEvent {
    pub project_id: String,
    pub workspace_id: String,
    pub previous_state: Option<WorkspaceState>,
    pub state: WorkspaceState,
    pub device_id: Option<String>,
    pub last_seen_head: Option<String>,
    pub last_checkpoint_id: Option<String>,
}

impl TypedEventPayload for WorkspaceStateChangedEvent {
    fn event_type(&self) -> EventType {
        EventType::WorkspaceStateChanged
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotLocalCreatedEvent {
    pub project_id: String,
    pub snapshot_id: String,
    pub snapshot_sequence_number: i64,
    pub session_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
    pub label: Option<String>,
    pub pinned: bool,
    pub state_hash: String,
    pub created_at_unix_seconds: u64,
}

impl SnapshotLocalCreatedEvent {
    pub fn from_snapshot(snapshot: &StoredSnapshot) -> Self {
        Self {
            project_id: snapshot.project_id.clone(),
            snapshot_id: snapshot.snapshot_id.clone(),
            snapshot_sequence_number: snapshot.sequence_number,
            session_id: snapshot.session_id.clone(),
            parent_snapshot_id: snapshot.parent_snapshot_id.clone(),
            label: snapshot.label.clone(),
            pinned: snapshot.pinned,
            state_hash: snapshot.metadata.state_hash.clone(),
            created_at_unix_seconds: snapshot.created_at_unix_seconds,
        }
    }
}

impl TypedEventPayload for SnapshotLocalCreatedEvent {
    fn event_type(&self) -> EventType {
        EventType::SnapshotLocalCreated
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotApplyStartedEvent {
    pub project_id: String,
    pub snapshot_id: String,
    pub target_workspace_id: Option<String>,
    pub dry_run: bool,
}

impl TypedEventPayload for SnapshotApplyStartedEvent {
    fn event_type(&self) -> EventType {
        EventType::SnapshotApplyStarted
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotApplyVerifiedEvent {
    pub project_id: String,
    pub snapshot_id: String,
    pub target_workspace_id: Option<String>,
    pub verification: VerificationDetails,
}

impl TypedEventPayload for SnapshotApplyVerifiedEvent {
    fn event_type(&self) -> EventType {
        EventType::SnapshotApplyVerified
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityBlockedEvent {
    pub code: String,
    pub title: String,
    pub detail: String,
    pub action: Option<String>,
    pub project_id: Option<String>,
    pub workspace_id: Option<String>,
    pub safe_actions: Vec<String>,
}

impl TypedEventPayload for SecurityBlockedEvent {
    fn event_type(&self) -> EventType {
        EventType::SecurityBlocked
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaWarningEvent {
    pub quota: String,
    pub scope: String,
    pub used: u64,
    pub limit: Option<u64>,
    pub unit: String,
    pub project_id: Option<String>,
    pub workspace_id: Option<String>,
    pub detail: Option<String>,
}

impl TypedEventPayload for QuotaWarningEvent {
    fn event_type(&self) -> EventType {
        EventType::QuotaWarning
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub schema_version: u32,
    pub sequence: EventSequence,
    pub occurred_at_unix_millis: EventTimestampMillis,
    #[serde(rename = "type")]
    pub event_type: EventType,
    pub payload: Value,
}

impl EventEnvelope {
    pub fn new(sequence: EventSequence, event_type: EventType, payload: Value) -> Self {
        Self::new_at(sequence, EventTimestampMillis::now(), event_type, payload)
    }

    pub fn new_at(
        sequence: EventSequence,
        occurred_at_unix_millis: EventTimestampMillis,
        event_type: EventType,
        payload: Value,
    ) -> Self {
        Self {
            schema_version: EVENT_SCHEMA_VERSION,
            sequence,
            occurred_at_unix_millis,
            event_type,
            payload,
        }
    }

    pub fn with_payload<T: Serialize>(
        sequence: EventSequence,
        event_type: EventType,
        payload: T,
    ) -> Result<Self> {
        Self::with_payload_at(sequence, EventTimestampMillis::now(), event_type, payload)
    }

    pub fn with_payload_at<T: Serialize>(
        sequence: EventSequence,
        occurred_at_unix_millis: EventTimestampMillis,
        event_type: EventType,
        payload: T,
    ) -> Result<Self> {
        Ok(Self::new_at(
            sequence,
            occurred_at_unix_millis,
            event_type,
            serde_json::to_value(payload)?,
        ))
    }

    pub fn with_typed_payload<T: TypedEventPayload>(
        sequence: EventSequence,
        payload: T,
    ) -> Result<Self> {
        Self::with_typed_payload_at(sequence, EventTimestampMillis::now(), payload)
    }

    pub fn with_typed_payload_at<T: TypedEventPayload>(
        sequence: EventSequence,
        occurred_at_unix_millis: EventTimestampMillis,
        payload: T,
    ) -> Result<Self> {
        let event_type = payload.event_type();
        Self::with_payload_at(sequence, occurred_at_unix_millis, event_type, payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn envelope_serializes_stable_fields() {
        let envelope = EventEnvelope::new_at(
            EventSequence::new(7).unwrap(),
            EventTimestampMillis::new(1_700_000_000_123),
            EventType::WorkspaceStateChanged,
            json!({
                "project_id": "12345678",
                "workspace_id": "ws_abc",
                "state": "active"
            }),
        );

        let encoded = serde_json::to_value(&envelope).unwrap();
        assert_eq!(encoded["schema_version"], EVENT_SCHEMA_VERSION);
        assert_eq!(encoded["sequence"], 7);
        assert_eq!(encoded["occurred_at_unix_millis"], 1_700_000_000_123u64);
        assert_eq!(encoded["type"], "workspace.state.changed");
        assert_eq!(encoded["payload"]["workspace_id"], "ws_abc");

        let decoded: EventEnvelope = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn sequence_rejects_zero_and_increments_monotonically() {
        assert_eq!(EventSequence::new(0), None);

        let sequencer = EventSequencer::new();
        assert_eq!(sequencer.next().unwrap().get(), 1);
        assert_eq!(sequencer.next().unwrap().get(), 2);
        assert_eq!(sequencer.next().unwrap().get(), 3);

        let resumed = EventSequencer::starting_after(EventSequence::new(41).unwrap()).unwrap();
        assert_eq!(resumed.next().unwrap().get(), 42);

        let err = serde_json::from_value::<EventSequence>(json!(0)).unwrap_err();
        assert!(
            err.to_string().contains("greater than 0"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn timestamp_uses_unix_millis_value() {
        let timestamp = EventTimestampMillis::new(1234);
        assert_eq!(timestamp.get(), 1234);
        assert!(EventTimestampMillis::now().get() > 0);
    }

    #[test]
    fn replay_cursor_filters_events_after_sequence() {
        let first = EventSequence::new(1).unwrap();
        let second = EventSequence::new(2).unwrap();
        let third = EventSequence::new(3).unwrap();

        assert!(EventReplayCursor::from_start().accepts(first));

        let cursor = EventReplayCursor::after(second);
        assert!(!cursor.accepts(first));
        assert!(!cursor.accepts(second));
        assert!(cursor.accepts(third));

        let encoded = serde_json::to_value(cursor).unwrap();
        assert_eq!(encoded["after_sequence"], 2);

        let decoded: EventReplayCursor = serde_json::from_value(json!({
            "after_sequence": 2
        }))
        .unwrap();
        assert_eq!(decoded, cursor);

        let err = serde_json::from_value::<EventReplayCursor>(json!({
            "after_sequence": 0
        }))
        .unwrap_err();
        assert!(
            err.to_string().contains("greater than 0"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn typed_payload_constructor_uses_json_payload() {
        #[derive(Serialize)]
        struct Payload<'a> {
            project_id: &'a str,
        }

        let envelope = EventEnvelope::with_payload_at(
            EventSequence::new(1).unwrap(),
            EventTimestampMillis::new(99),
            EventType::QuotaWarning,
            Payload {
                project_id: "12345678",
            },
        )
        .unwrap();

        assert_eq!(envelope.payload["project_id"], "12345678");
        assert_eq!(envelope.occurred_at_unix_millis.get(), 99);
    }

    #[test]
    fn typed_event_payloads_select_stable_event_names() {
        let workspace = EventEnvelope::with_typed_payload_at(
            EventSequence::new(1).unwrap(),
            EventTimestampMillis::new(10),
            WorkspaceStateChangedEvent {
                project_id: "12345678".to_string(),
                workspace_id: "w_source".to_string(),
                previous_state: Some(WorkspaceState::Inactive),
                state: WorkspaceState::Active,
                device_id: Some("device-a".to_string()),
                last_seen_head: Some("abc123".to_string()),
                last_checkpoint_id: Some("s1_checkpoint".to_string()),
            },
        )
        .unwrap();
        let encoded = serde_json::to_value(workspace).unwrap();
        assert_eq!(encoded["type"], "workspace.state.changed");
        assert_eq!(encoded["payload"]["previous_state"], "inactive");
        assert_eq!(encoded["payload"]["state"], "active");

        let started = EventEnvelope::with_typed_payload_at(
            EventSequence::new(2).unwrap(),
            EventTimestampMillis::new(20),
            SnapshotApplyStartedEvent {
                project_id: "12345678".to_string(),
                snapshot_id: "s1_abc".to_string(),
                target_workspace_id: Some("w_target".to_string()),
                dry_run: false,
            },
        )
        .unwrap();
        let encoded = serde_json::to_value(started).unwrap();
        assert_eq!(encoded["type"], "snapshot.apply.started");
        assert_eq!(encoded["payload"]["target_workspace_id"], "w_target");

        let verified = EventEnvelope::with_typed_payload_at(
            EventSequence::new(3).unwrap(),
            EventTimestampMillis::new(30),
            SnapshotApplyVerifiedEvent {
                project_id: "12345678".to_string(),
                snapshot_id: "s1_abc".to_string(),
                target_workspace_id: Some("w_target".to_string()),
                verification: VerificationDetails {
                    head_oid: "head".to_string(),
                    index_tree_oid: "index".to_string(),
                    work_tree_oid: "work".to_string(),
                    state_hash: "state-hash".to_string(),
                    included_untracked: vec!["notes.md".to_string()],
                    excluded_paths: vec![".env".to_string()],
                },
            },
        )
        .unwrap();
        let encoded = serde_json::to_value(verified).unwrap();
        assert_eq!(encoded["type"], "snapshot.apply.verified");
        assert_eq!(
            encoded["payload"]["verification"]["state_hash"],
            "state-hash"
        );

        let security = EventEnvelope::with_typed_payload_at(
            EventSequence::new(4).unwrap(),
            EventTimestampMillis::new(40),
            SecurityBlockedEvent {
                code: "DR-SECURITY-BLOCKED".to_string(),
                title: "Security block".to_string(),
                detail: "secret-like path was excluded".to_string(),
                action: Some("checkpoint".to_string()),
                project_id: Some("12345678".to_string()),
                workspace_id: None,
                safe_actions: vec!["Review the excluded path.".to_string()],
            },
        )
        .unwrap();
        let encoded = serde_json::to_value(security).unwrap();
        assert_eq!(encoded["type"], "security.blocked");
        assert_eq!(
            encoded["payload"]["safe_actions"][0],
            "Review the excluded path."
        );

        let quota = EventEnvelope::with_typed_payload_at(
            EventSequence::new(5).unwrap(),
            EventTimestampMillis::new(50),
            QuotaWarningEvent {
                quota: "snapshot-store".to_string(),
                scope: "project".to_string(),
                used: 900,
                limit: Some(1000),
                unit: "bytes".to_string(),
                project_id: Some("12345678".to_string()),
                workspace_id: None,
                detail: Some("snapshot store is nearing its limit".to_string()),
            },
        )
        .unwrap();
        let encoded = serde_json::to_value(quota).unwrap();
        assert_eq!(encoded["type"], "quota.warning");
        assert_eq!(encoded["payload"]["limit"], 1000);
    }

    #[test]
    fn snapshot_local_created_payload_comes_from_stored_snapshot() {
        let metadata: crate::SnapshotMetadata =
            serde_json::from_str(include_str!("../tests/fixtures/snapshot_metadata_v1.json"))
                .unwrap();
        let stored = StoredSnapshot {
            snapshot_id: metadata.snapshot_id.clone(),
            project_id: metadata.project_id.clone(),
            session_id: Some("session-a".to_string()),
            parent_snapshot_id: Some("s1_parent".to_string()),
            sequence_number: 11,
            pinned: true,
            label: Some("manual".to_string()),
            metadata,
            created_at_unix_seconds: 1_700_000_000,
        };

        let payload = SnapshotLocalCreatedEvent::from_snapshot(&stored);
        assert_eq!(payload.project_id, stored.project_id);
        assert_eq!(payload.snapshot_id, stored.snapshot_id);
        assert_eq!(payload.snapshot_sequence_number, 11);
        assert_eq!(payload.parent_snapshot_id.as_deref(), Some("s1_parent"));
        assert!(payload.pinned);

        let envelope = EventEnvelope::with_typed_payload_at(
            EventSequence::new(12).unwrap(),
            EventTimestampMillis::new(120),
            payload,
        )
        .unwrap();
        let encoded = serde_json::to_value(envelope).unwrap();
        assert_eq!(encoded["type"], "snapshot.local.created");
        assert_eq!(encoded["payload"]["snapshot_sequence_number"], 11);
        assert_eq!(encoded["payload"]["created_at_unix_seconds"], 1_700_000_000);
    }
}
