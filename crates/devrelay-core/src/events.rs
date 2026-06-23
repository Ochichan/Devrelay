//! Event stream envelope types shared by the agent and local clients.
//!
//! M2 events are ordered by a single local agent sequence. The sequence is the
//! replay cursor boundary; timestamps are informational and must not be used to
//! infer ordering.

use crate::Result;
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
}
