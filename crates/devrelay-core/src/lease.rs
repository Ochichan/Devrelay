use crate::{DevRelayError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LeaseState {
    Active,
    HandoffPending,
    Committing,
    Inactive,
    Forked,
    Archived,
}

impl LeaseState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::HandoffPending => "handoff-pending",
            Self::Committing => "committing",
            Self::Inactive => "inactive",
            Self::Forked => "forked",
            Self::Archived => "archived",
        }
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Active, Self::Active)
                | (Self::Active, Self::HandoffPending)
                | (Self::Active, Self::Committing)
                | (Self::Active, Self::Inactive)
                | (Self::Active, Self::Archived)
                | (Self::HandoffPending, Self::Active)
                | (Self::HandoffPending, Self::HandoffPending)
                | (Self::HandoffPending, Self::Committing)
                | (Self::HandoffPending, Self::Archived)
                | (Self::Committing, Self::Active)
                | (Self::Committing, Self::Committing)
                | (Self::Committing, Self::Inactive)
                | (Self::Inactive, Self::Inactive)
                | (Self::Inactive, Self::Forked)
                | (Self::Inactive, Self::Archived)
                | (Self::Forked, Self::Forked)
                | (Self::Forked, Self::Archived)
                | (Self::Archived, Self::Archived)
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LeaseRecord {
    pub lease_id: String,
    pub project_id: String,
    pub session_id: String,
    pub state: LeaseState,
    pub epoch: u64,
    pub holder_device_id: Option<String>,
    pub latest_snapshot_id: Option<String>,
    pub handoff_id: Option<String>,
}

impl LeaseRecord {
    pub fn validate_transition_to(&self, next: &Self) -> Result<()> {
        if self.lease_id != next.lease_id {
            return Err(DevRelayError::Config(
                "lease transition cannot change lease_id".to_string(),
            ));
        }
        if self.project_id != next.project_id {
            return Err(DevRelayError::Config(
                "lease transition cannot change project_id".to_string(),
            ));
        }
        if self.session_id != next.session_id {
            return Err(DevRelayError::Config(
                "lease transition cannot change session_id".to_string(),
            ));
        }
        if !self.state.can_transition_to(next.state) {
            return Err(DevRelayError::Config(format!(
                "illegal lease transition {} -> {}",
                self.state.as_str(),
                next.state.as_str()
            )));
        }
        if next.epoch < self.epoch {
            return Err(DevRelayError::Config(format!(
                "lease epoch cannot decrease from {} to {}",
                self.epoch, next.epoch
            )));
        }
        if self.state == LeaseState::Committing
            && next.state == LeaseState::Active
            && next.epoch == self.epoch
        {
            return Err(DevRelayError::Config(
                "committed lease handoff must increment epoch".to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease(state: LeaseState, epoch: u64) -> LeaseRecord {
        LeaseRecord {
            lease_id: "lease-1".to_string(),
            project_id: "project-1".to_string(),
            session_id: "session-1".to_string(),
            state,
            epoch,
            holder_device_id: Some("device-a".to_string()),
            latest_snapshot_id: Some("s1_initial".to_string()),
            handoff_id: None,
        }
    }

    #[test]
    fn allows_legal_lease_transitions() {
        lease(LeaseState::Active, 1)
            .validate_transition_to(&lease(LeaseState::HandoffPending, 1))
            .unwrap();
        lease(LeaseState::HandoffPending, 1)
            .validate_transition_to(&lease(LeaseState::Committing, 1))
            .unwrap();
        lease(LeaseState::Committing, 1)
            .validate_transition_to(&lease(LeaseState::Active, 2))
            .unwrap();
        lease(LeaseState::Inactive, 2)
            .validate_transition_to(&lease(LeaseState::Forked, 2))
            .unwrap();
        lease(LeaseState::Forked, 2)
            .validate_transition_to(&lease(LeaseState::Archived, 2))
            .unwrap();
    }

    #[test]
    fn rejects_illegal_lease_transitions() {
        let err = lease(LeaseState::Active, 1)
            .validate_transition_to(&lease(LeaseState::Forked, 1))
            .unwrap_err();
        assert!(err.to_string().contains("illegal lease transition"));

        let err = lease(LeaseState::Archived, 2)
            .validate_transition_to(&lease(LeaseState::Active, 3))
            .unwrap_err();
        assert!(err.to_string().contains("illegal lease transition"));
    }

    #[test]
    fn enforces_epoch_monotonicity() {
        let err = lease(LeaseState::Active, 2)
            .validate_transition_to(&lease(LeaseState::HandoffPending, 1))
            .unwrap_err();
        assert!(err.to_string().contains("cannot decrease"));

        let err = lease(LeaseState::Committing, 2)
            .validate_transition_to(&lease(LeaseState::Active, 2))
            .unwrap_err();
        assert!(err.to_string().contains("must increment epoch"));
    }
}
