//! Durable security and control-plane audit event types.
//!
//! Audit records are append-only metadata. They intentionally duplicate the
//! event stream at durable boundaries so pairing, publishing, applying, and
//! lease-transfer decisions can be inspected after an agent restart.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const AUDIT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditEventType {
    #[serde(rename = "device.paired")]
    DevicePaired,
    #[serde(rename = "device.revoked")]
    DeviceRevoked,
    #[serde(rename = "snapshot.published")]
    SnapshotPublished,
    #[serde(rename = "snapshot.applied")]
    SnapshotApplied,
    #[serde(rename = "lease.transferred")]
    LeaseTransferred,
    #[serde(rename = "editor.context.updated")]
    EditorContextUpdated,
    #[serde(rename = "editor.restore.acked")]
    EditorRestoreAcked,
    #[serde(rename = "command.approved")]
    CommandApproved,
    #[serde(rename = "security.blocked")]
    SecurityBlocked,
}

impl AuditEventType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DevicePaired => "device.paired",
            Self::DeviceRevoked => "device.revoked",
            Self::SnapshotPublished => "snapshot.published",
            Self::SnapshotApplied => "snapshot.applied",
            Self::LeaseTransferred => "lease.transferred",
            Self::EditorContextUpdated => "editor.context.updated",
            Self::EditorRestoreAcked => "editor.restore.acked",
            Self::CommandApproved => "command.approved",
            Self::SecurityBlocked => "security.blocked",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "device.revoked" => Self::DeviceRevoked,
            "snapshot.published" => Self::SnapshotPublished,
            "snapshot.applied" => Self::SnapshotApplied,
            "lease.transferred" => Self::LeaseTransferred,
            "editor.context.updated" => Self::EditorContextUpdated,
            "editor.restore.acked" => Self::EditorRestoreAcked,
            "command.approved" => Self::CommandApproved,
            "security.blocked" => Self::SecurityBlocked,
            _ => Self::DevicePaired,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuditOutcome {
    Succeeded,
    Failed,
    Blocked,
}

impl AuditOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "failed" => Self::Failed,
            "blocked" => Self::Blocked,
            _ => Self::Succeeded,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEventInput {
    #[serde(rename = "type")]
    pub event_type: AuditEventType,
    pub outcome: AuditOutcome,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_id: Option<String>,
    #[serde(default = "empty_object")]
    pub detail: Value,
}

impl AuditEventInput {
    pub fn new(
        event_type: AuditEventType,
        outcome: AuditOutcome,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            event_type,
            outcome,
            summary: summary.into(),
            project_id: None,
            actor_device_id: None,
            target_device_id: None,
            session_id: None,
            snapshot_id: None,
            lease_id: None,
            handoff_id: None,
            detail: empty_object(),
        }
    }

    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = detail;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEventRecord {
    pub schema_version: u32,
    pub audit_id: i64,
    #[serde(rename = "type")]
    pub event_type: AuditEventType,
    pub outcome: AuditOutcome,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_id: Option<String>,
    pub detail: Value,
    pub created_at_unix_seconds: u64,
}

fn empty_object() -> Value {
    serde_json::json!({})
}
