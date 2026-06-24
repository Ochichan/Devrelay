//! Environment hydration state machine.
//!
//! Hydration state is separate from code handoff state. It records whether the
//! local environment around a verified workspace is ready, retryable, or failed
//! without mutating Git state.

use crate::{DevRelayError, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HydrationState {
    Cold,
    MetadataReady,
    CacheReady,
    ShellReady,
    AppReady,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HydrationTransition {
    MetadataPrepared,
    CachePrepared,
    ShellPrepared,
    AppPrepared,
    Failed,
    Retry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HydrationStateRecord {
    pub project_id: String,
    pub workspace_id: Option<String>,
    pub state: HydrationState,
    pub attempt: u32,
    pub failure: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_unix_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_at_unix_seconds: Option<u64>,
    pub updated_at_unix_seconds: u64,
}

impl HydrationStateRecord {
    pub fn new(
        project_id: impl Into<String>,
        workspace_id: Option<String>,
        updated_at_unix_seconds: u64,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            workspace_id,
            state: HydrationState::Cold,
            attempt: 1,
            failure: None,
            started_at_unix_seconds: Some(updated_at_unix_seconds),
            ready_at_unix_seconds: None,
            updated_at_unix_seconds,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HydrationProgress {
    pub project_id: String,
    pub workspace_id: Option<String>,
    pub previous_state: Option<HydrationState>,
    pub state: HydrationState,
    pub transition: HydrationTransition,
    pub attempt: u32,
    pub failure: Option<String>,
    pub updated_at_unix_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrationStateMachine {
    record: HydrationStateRecord,
}

impl HydrationStateMachine {
    pub fn new(record: HydrationStateRecord) -> Self {
        Self { record }
    }

    pub fn record(&self) -> &HydrationStateRecord {
        &self.record
    }

    pub fn into_record(self) -> HydrationStateRecord {
        self.record
    }

    pub fn apply(
        &mut self,
        transition: HydrationTransition,
        updated_at_unix_seconds: u64,
        failure: Option<String>,
    ) -> Result<HydrationProgress> {
        let previous_state = self.record.state;
        let next = match (previous_state, transition) {
            (HydrationState::Cold, HydrationTransition::MetadataPrepared) => {
                HydrationState::MetadataReady
            }
            (HydrationState::MetadataReady, HydrationTransition::CachePrepared) => {
                HydrationState::CacheReady
            }
            (HydrationState::CacheReady, HydrationTransition::ShellPrepared) => {
                HydrationState::ShellReady
            }
            (HydrationState::ShellReady, HydrationTransition::AppPrepared) => {
                HydrationState::AppReady
            }
            (HydrationState::Failed, HydrationTransition::Retry) => HydrationState::Cold,
            (_, HydrationTransition::Failed) => HydrationState::Failed,
            _ => {
                return Err(DevRelayError::Config(format!(
                    "illegal hydration transition {:?} -> {:?}",
                    previous_state, transition
                )));
            }
        };

        if transition == HydrationTransition::Retry {
            self.record.attempt = self.record.attempt.saturating_add(1);
            self.record.failure = None;
            self.record.started_at_unix_seconds = Some(updated_at_unix_seconds);
            self.record.ready_at_unix_seconds = None;
        } else if transition == HydrationTransition::Failed {
            self.record.failure = failure.clone();
            if self.record.started_at_unix_seconds.is_none() {
                self.record.started_at_unix_seconds = Some(updated_at_unix_seconds);
            }
        } else {
            self.record.failure = None;
        }
        if matches!(
            transition,
            HydrationTransition::ShellPrepared | HydrationTransition::AppPrepared
        ) && self.record.ready_at_unix_seconds.is_none()
        {
            self.record.ready_at_unix_seconds = Some(updated_at_unix_seconds);
        }
        self.record.state = next;
        self.record.updated_at_unix_seconds = updated_at_unix_seconds;

        Ok(HydrationProgress {
            project_id: self.record.project_id.clone(),
            workspace_id: self.record.workspace_id.clone(),
            previous_state: Some(previous_state),
            state: self.record.state,
            transition,
            attempt: self.record.attempt,
            failure: self.record.failure.clone(),
            updated_at_unix_seconds,
        })
    }
}

pub fn save_hydration_state(path: &Path, record: &HydrationStateRecord) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(record)?)?;
    Ok(())
}

pub fn load_hydration_state(path: &Path) -> Result<HydrationStateRecord> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hydration_state_machine_reaches_app_ready_in_order() {
        let mut machine = HydrationStateMachine::new(HydrationStateRecord::new(
            "project-a",
            Some("workspace-a".to_string()),
            10,
        ));

        let metadata = machine
            .apply(HydrationTransition::MetadataPrepared, 11, None)
            .unwrap();
        assert_eq!(metadata.state, HydrationState::MetadataReady);
        assert_eq!(metadata.previous_state, Some(HydrationState::Cold));

        machine
            .apply(HydrationTransition::CachePrepared, 12, None)
            .unwrap();
        machine
            .apply(HydrationTransition::ShellPrepared, 13, None)
            .unwrap();
        let app = machine
            .apply(HydrationTransition::AppPrepared, 14, None)
            .unwrap();

        assert_eq!(app.state, HydrationState::AppReady);
        assert_eq!(machine.record().failure, None);
        assert_eq!(machine.record().started_at_unix_seconds, Some(10));
        assert_eq!(machine.record().ready_at_unix_seconds, Some(13));
    }

    #[test]
    fn hydration_state_machine_blocks_invalid_transitions_and_retries_failure() {
        let mut machine =
            HydrationStateMachine::new(HydrationStateRecord::new("project-a", None, 10));

        let err = machine
            .apply(HydrationTransition::ShellPrepared, 11, None)
            .unwrap_err();
        assert!(err.to_string().contains("illegal hydration transition"));

        let failed = machine
            .apply(
                HydrationTransition::Failed,
                12,
                Some("nix failed".to_string()),
            )
            .unwrap();
        assert_eq!(failed.state, HydrationState::Failed);
        assert_eq!(failed.failure.as_deref(), Some("nix failed"));

        let retry = machine.apply(HydrationTransition::Retry, 13, None).unwrap();
        assert_eq!(retry.state, HydrationState::Cold);
        assert_eq!(retry.attempt, 2);
        assert_eq!(retry.failure, None);
        assert_eq!(machine.record().started_at_unix_seconds, Some(13));
        assert_eq!(machine.record().ready_at_unix_seconds, None);
    }

    #[test]
    fn hydration_state_persists_as_json() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("hydration/state.json");
        let mut record = HydrationStateRecord::new("project-a", Some("workspace-a".to_string()), 1);
        record.state = HydrationState::ShellReady;
        record.attempt = 3;

        save_hydration_state(&path, &record).unwrap();
        let loaded = load_hydration_state(&path).unwrap();

        assert_eq!(loaded, record);
    }

    #[test]
    fn hydration_state_loads_legacy_json_without_duration_fields() {
        let legacy = r#"{
  "project_id": "project-a",
  "workspace_id": "workspace-a",
  "state": "shell-ready",
  "attempt": 2,
  "failure": null,
  "updated_at_unix_seconds": 42
}"#;

        let record: HydrationStateRecord = serde_json::from_str(legacy).unwrap();

        assert_eq!(record.state, HydrationState::ShellReady);
        assert_eq!(record.started_at_unix_seconds, None);
        assert_eq!(record.ready_at_unix_seconds, None);
    }
}
