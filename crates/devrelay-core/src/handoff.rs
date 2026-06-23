use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub const HANDOFF_ID_PREFIX: &str = "ho_";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HandoffState {
    TargetPrepare,
    TargetVerified,
    SourceReady,
    Committed,
    Aborted,
}

impl HandoffState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TargetPrepare => "target-prepare",
            Self::TargetVerified => "target-verified",
            Self::SourceReady => "source-ready",
            Self::Committed => "committed",
            Self::Aborted => "aborted",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "target-verified" => Self::TargetVerified,
            "source-ready" => Self::SourceReady,
            "committed" => Self::Committed,
            "aborted" => Self::Aborted,
            _ => Self::TargetPrepare,
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Committed | Self::Aborted)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HandoffJournalPhase {
    Begin,
    TargetPrepare,
    TargetApply,
    TargetVerified,
    SourceReady,
    LeaseCommitted,
    Aborted,
}

impl HandoffJournalPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Begin => "begin",
            Self::TargetPrepare => "target-prepare",
            Self::TargetApply => "target-apply",
            Self::TargetVerified => "target-verified",
            Self::SourceReady => "source-ready",
            Self::LeaseCommitted => "lease-committed",
            Self::Aborted => "aborted",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "target-prepare" => Self::TargetPrepare,
            "target-apply" => Self::TargetApply,
            "target-verified" => Self::TargetVerified,
            "source-ready" => Self::SourceReady,
            "lease-committed" => Self::LeaseCommitted,
            "aborted" => Self::Aborted,
            _ => Self::Begin,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandoffRecord {
    pub handoff_id: String,
    pub lease_id: String,
    pub project_id: String,
    pub expected_epoch: u64,
    pub source_device_id: String,
    pub target_device_id: String,
    pub source_generation: String,
    pub expires_at_unix_seconds: u64,
    pub state: HandoffState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandoffJournalRecord {
    pub journal_id: i64,
    pub handoff_id: String,
    pub lease_id: String,
    pub project_id: String,
    pub phase: HandoffJournalPhase,
    pub detail_json: String,
    pub created_at_unix_seconds: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HandoffRecoveryOutcome {
    WaitingForTarget,
    Committed,
    AbortedExpired,
    AlreadyCommitted,
    AlreadyAborted,
}

pub fn generate_handoff_id() -> String {
    let seed = format!("{}\0{}", std::process::id(), unix_now_nanos());
    let digest = blake3::hash(seed.as_bytes());
    format!("{HANDOFF_ID_PREFIX}{}", &digest.to_hex()[..24])
}

fn unix_now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
