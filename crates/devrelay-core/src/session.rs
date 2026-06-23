use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SESSION_ID_PREFIX: &str = "se_";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredSession {
    pub session_id: String,
    pub project_id: String,
    pub name: String,
    pub parent_session_id: Option<String>,
    pub active_workspace_id: Option<String>,
    pub state: SessionState,
    pub archived_at_unix_seconds: Option<u64>,
    pub created_at_unix_seconds: u64,
    pub updated_at_unix_seconds: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SessionState {
    Active,
    Archived,
    Fork,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
            Self::Fork => "fork",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "archived" => Self::Archived,
            "fork" => Self::Fork,
            _ => Self::Active,
        }
    }
}

pub fn generate_session_id() -> String {
    let seed = format!("{}\0{}", std::process::id(), unix_now_nanos());
    let digest = blake3::hash(seed.as_bytes());
    format!("{SESSION_ID_PREFIX}{}", &digest.to_hex()[..24])
}

pub fn unix_now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn unix_now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
