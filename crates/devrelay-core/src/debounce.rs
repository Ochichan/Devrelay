//! Adaptive debounce scheduling for background protection.
//!
//! The debouncer only decides when work should be attempted. Callers still own
//! Git status, snapshot creation, publishing, and error handling.

use crate::watcher::CoalescedWorkspaceChange;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackgroundDebouncePolicy {
    pub first_event_quiet: Duration,
    pub min_checkpoint_interval: Duration,
    pub max_dirty_interval: Duration,
    pub publish_quiet: Duration,
    pub max_publish_interval: Duration,
}

impl Default for BackgroundDebouncePolicy {
    fn default() -> Self {
        Self {
            first_event_quiet: Duration::from_secs(2),
            min_checkpoint_interval: Duration::from_secs(30),
            max_dirty_interval: Duration::from_secs(5 * 60),
            publish_quiet: Duration::from_secs(10),
            max_publish_interval: Duration::from_secs(2 * 60),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebounceFlushReason {
    QuietWindow,
    MaxDirtyInterval,
    PublishQuietWindow,
    MaxPublishInterval,
    ExplicitCheckpoint,
    Handoff,
    SleepOrLock,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebouncedCheckpoint {
    pub workspace_id: String,
    pub source_generation: u64,
    pub reason: DebounceFlushReason,
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebouncedPublish {
    pub workspace_id: String,
    pub source_generation: u64,
    pub reason: DebounceFlushReason,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DebounceDrain {
    pub checkpoints: Vec<DebouncedCheckpoint>,
    pub publishes: Vec<DebouncedPublish>,
}

impl DebounceDrain {
    pub fn is_empty(&self) -> bool {
        self.checkpoints.is_empty() && self.publishes.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct AdaptiveDebouncer {
    policy: BackgroundDebouncePolicy,
    workspaces: BTreeMap<String, WorkspaceDebounceState>,
}

impl AdaptiveDebouncer {
    pub fn new(policy: BackgroundDebouncePolicy) -> Self {
        Self {
            policy,
            workspaces: BTreeMap::new(),
        }
    }

    pub fn policy(&self) -> BackgroundDebouncePolicy {
        self.policy
    }

    pub fn observe_change(&mut self, change: CoalescedWorkspaceChange, now: Duration) {
        if change.paths.is_empty() {
            return;
        }

        let state = self.workspaces.entry(change.workspace_id).or_default();
        if state.first_dirty_at.is_none() {
            state.first_dirty_at = Some(now);
        }
        state.last_dirty_event_at = Some(now);
        state.source_generation = state.source_generation.max(change.source_generation);
        state.pending_paths.extend(change.paths);
    }

    pub fn record_checkpoint_completed(
        &mut self,
        workspace_id: impl Into<String>,
        source_generation: u64,
        now: Duration,
    ) {
        let state = self.workspaces.entry(workspace_id.into()).or_default();
        if state.first_unpublished_at.is_none() {
            state.first_unpublished_at = Some(now);
        }
        state.last_unpublished_at = Some(now);
        state.publish_generation = Some(
            state
                .publish_generation
                .unwrap_or_default()
                .max(source_generation),
        );
    }

    pub fn drain_due(&mut self, now: Duration) -> DebounceDrain {
        let workspace_ids: Vec<_> = self.workspaces.keys().cloned().collect();
        let mut drain = DebounceDrain::default();
        for workspace_id in workspace_ids {
            if let Some(checkpoint) = self.take_due_checkpoint(&workspace_id, now) {
                drain.checkpoints.push(checkpoint);
            }
            if let Some(publish) = self.take_due_publish(&workspace_id, now) {
                drain.publishes.push(publish);
            }
        }
        drain
    }

    pub fn flush_all(&mut self, now: Duration, reason: DebounceFlushReason) -> DebounceDrain {
        let workspace_ids: Vec<_> = self.workspaces.keys().cloned().collect();
        let mut drain = DebounceDrain::default();
        for workspace_id in workspace_ids {
            if let Some(checkpoint) = self.take_checkpoint(&workspace_id, now, reason) {
                drain.checkpoints.push(checkpoint);
            }
            if let Some(publish) = self.take_publish(&workspace_id, now, reason) {
                drain.publishes.push(publish);
            }
        }
        drain
    }

    pub fn flush_for_sleep_or_lock(&mut self, now: Duration) -> DebounceDrain {
        self.flush_all(now, DebounceFlushReason::SleepOrLock)
    }

    fn take_due_checkpoint(
        &mut self,
        workspace_id: &str,
        now: Duration,
    ) -> Option<DebouncedCheckpoint> {
        let state = self.workspaces.get(workspace_id)?;
        if state.pending_paths.is_empty() {
            return None;
        }

        let first_dirty_at = state.first_dirty_at?;
        let last_dirty_event_at = state.last_dirty_event_at.unwrap_or(first_dirty_at);
        let since_first_dirty = elapsed(now, first_dirty_at);
        let since_last_event = elapsed(now, last_dirty_event_at);
        let min_interval_satisfied = state
            .last_checkpoint_at
            .map(|last| elapsed(now, last) >= self.policy.min_checkpoint_interval)
            .unwrap_or(true);

        let reason = if since_first_dirty >= self.policy.max_dirty_interval {
            DebounceFlushReason::MaxDirtyInterval
        } else if since_last_event >= self.policy.first_event_quiet && min_interval_satisfied {
            DebounceFlushReason::QuietWindow
        } else {
            return None;
        };

        self.take_checkpoint(workspace_id, now, reason)
    }

    fn take_due_publish(&mut self, workspace_id: &str, now: Duration) -> Option<DebouncedPublish> {
        let state = self.workspaces.get(workspace_id)?;
        state.publish_generation?;

        let first_unpublished_at = state.first_unpublished_at?;
        let last_unpublished_at = state.last_unpublished_at.unwrap_or(first_unpublished_at);
        let since_first_unpublished = elapsed(now, first_unpublished_at);
        let since_last_unpublished = elapsed(now, last_unpublished_at);

        let reason = if since_first_unpublished >= self.policy.max_publish_interval {
            DebounceFlushReason::MaxPublishInterval
        } else if since_last_unpublished >= self.policy.publish_quiet {
            DebounceFlushReason::PublishQuietWindow
        } else {
            return None;
        };

        self.take_publish(workspace_id, now, reason)
    }

    fn take_checkpoint(
        &mut self,
        workspace_id: &str,
        now: Duration,
        reason: DebounceFlushReason,
    ) -> Option<DebouncedCheckpoint> {
        let state = self.workspaces.get_mut(workspace_id)?;
        if state.pending_paths.is_empty() {
            return None;
        }

        let paths = std::mem::take(&mut state.pending_paths);
        state.first_dirty_at = None;
        state.last_dirty_event_at = None;
        state.last_checkpoint_at = Some(now);

        Some(DebouncedCheckpoint {
            workspace_id: workspace_id.to_string(),
            source_generation: state.source_generation,
            reason,
            paths: paths.into_iter().collect(),
        })
    }

    fn take_publish(
        &mut self,
        workspace_id: &str,
        now: Duration,
        reason: DebounceFlushReason,
    ) -> Option<DebouncedPublish> {
        let state = self.workspaces.get_mut(workspace_id)?;
        let source_generation = state.publish_generation.take()?;
        state.first_unpublished_at = None;
        state.last_unpublished_at = None;
        state.last_publish_at = Some(now);

        Some(DebouncedPublish {
            workspace_id: workspace_id.to_string(),
            source_generation,
            reason,
        })
    }
}

#[derive(Debug, Default)]
struct WorkspaceDebounceState {
    pending_paths: BTreeSet<PathBuf>,
    source_generation: u64,
    first_dirty_at: Option<Duration>,
    last_dirty_event_at: Option<Duration>,
    last_checkpoint_at: Option<Duration>,
    publish_generation: Option<u64>,
    first_unpublished_at: Option<Duration>,
    last_unpublished_at: Option<Duration>,
    last_publish_at: Option<Duration>,
}

fn elapsed(now: Duration, then: Duration) -> Duration {
    now.checked_sub(then).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seconds(value: u64) -> Duration {
        Duration::from_secs(value)
    }

    fn policy() -> BackgroundDebouncePolicy {
        BackgroundDebouncePolicy {
            first_event_quiet: seconds(5),
            min_checkpoint_interval: seconds(10),
            max_dirty_interval: seconds(20),
            publish_quiet: seconds(5),
            max_publish_interval: seconds(20),
        }
    }

    fn change(
        workspace_id: &str,
        source_generation: u64,
        paths: &[&str],
    ) -> CoalescedWorkspaceChange {
        CoalescedWorkspaceChange {
            workspace_id: workspace_id.to_string(),
            source_generation,
            paths: paths.iter().map(PathBuf::from).collect(),
        }
    }

    #[test]
    fn quiet_timer_waits_for_first_event_window_and_min_checkpoint_interval() {
        let mut debouncer = AdaptiveDebouncer::new(policy());
        debouncer.observe_change(change("w_main", 1, &["src/lib.rs"]), seconds(0));

        assert!(debouncer.drain_due(seconds(4)).is_empty());
        let first = debouncer.drain_due(seconds(5));
        assert_eq!(first.checkpoints.len(), 1);
        assert_eq!(
            first.checkpoints[0].reason,
            DebounceFlushReason::QuietWindow
        );

        debouncer.observe_change(change("w_main", 2, &["src/main.rs"]), seconds(6));
        assert!(debouncer.drain_due(seconds(11)).is_empty());

        let second = debouncer.drain_due(seconds(15));
        assert_eq!(second.checkpoints.len(), 1);
        assert_eq!(
            second.checkpoints[0].reason,
            DebounceFlushReason::QuietWindow
        );
        assert_eq!(second.checkpoints[0].source_generation, 2);
    }

    #[test]
    fn max_dirty_interval_flushes_even_when_events_keep_arriving() {
        let mut debouncer = AdaptiveDebouncer::new(policy());
        debouncer.observe_change(change("w_main", 1, &["a.txt"]), seconds(0));
        debouncer.observe_change(change("w_main", 2, &["b.txt"]), seconds(8));
        debouncer.observe_change(change("w_main", 3, &["c.txt"]), seconds(16));

        assert!(debouncer.drain_due(seconds(19)).is_empty());
        let drain = debouncer.drain_due(seconds(20));

        assert_eq!(drain.checkpoints.len(), 1);
        assert_eq!(
            drain.checkpoints[0].reason,
            DebounceFlushReason::MaxDirtyInterval
        );
        assert_eq!(drain.checkpoints[0].source_generation, 3);
        assert_eq!(
            drain.checkpoints[0].paths,
            vec![
                PathBuf::from("a.txt"),
                PathBuf::from("b.txt"),
                PathBuf::from("c.txt")
            ]
        );
    }

    #[test]
    fn publish_quiet_timer_waits_for_unpublished_checkpoint_quiet_window() {
        let mut debouncer = AdaptiveDebouncer::new(policy());
        debouncer.record_checkpoint_completed("w_main", 7, seconds(0));

        assert!(debouncer.drain_due(seconds(4)).is_empty());
        let drain = debouncer.drain_due(seconds(5));

        assert_eq!(drain.publishes.len(), 1);
        assert_eq!(
            drain.publishes[0].reason,
            DebounceFlushReason::PublishQuietWindow
        );
        assert_eq!(drain.publishes[0].source_generation, 7);
    }

    #[test]
    fn max_publish_interval_flushes_when_checkpoints_keep_arriving() {
        let mut custom = policy();
        custom.publish_quiet = seconds(30);
        custom.max_publish_interval = seconds(12);
        let mut debouncer = AdaptiveDebouncer::new(custom);

        debouncer.record_checkpoint_completed("w_main", 1, seconds(0));
        debouncer.record_checkpoint_completed("w_main", 2, seconds(8));

        assert!(debouncer.drain_due(seconds(11)).is_empty());
        let drain = debouncer.drain_due(seconds(12));

        assert_eq!(drain.publishes.len(), 1);
        assert_eq!(
            drain.publishes[0].reason,
            DebounceFlushReason::MaxPublishInterval
        );
        assert_eq!(drain.publishes[0].source_generation, 2);
    }

    #[test]
    fn explicit_checkpoint_and_handoff_flush_pending_work_immediately() {
        for reason in [
            DebounceFlushReason::ExplicitCheckpoint,
            DebounceFlushReason::Handoff,
        ] {
            let mut debouncer = AdaptiveDebouncer::new(policy());
            debouncer.observe_change(change("w_main", 1, &["src/lib.rs"]), seconds(0));

            let checkpoints = debouncer.flush_all(seconds(1), reason);
            assert_eq!(checkpoints.checkpoints.len(), 1);
            assert_eq!(checkpoints.checkpoints[0].reason, reason);
            assert!(checkpoints.publishes.is_empty());

            debouncer.record_checkpoint_completed("w_main", 1, seconds(1));
            let publishes = debouncer.flush_all(seconds(2), reason);
            assert!(publishes.checkpoints.is_empty());
            assert_eq!(publishes.publishes.len(), 1);
            assert_eq!(publishes.publishes[0].reason, reason);
        }
    }

    #[test]
    fn sleep_or_lock_flushes_pending_work_immediately() {
        let mut debouncer = AdaptiveDebouncer::new(policy());
        debouncer.observe_change(change("w_main", 1, &["src/lib.rs"]), seconds(0));

        let checkpoints = debouncer.flush_for_sleep_or_lock(seconds(1));
        assert_eq!(checkpoints.checkpoints.len(), 1);
        assert_eq!(
            checkpoints.checkpoints[0].reason,
            DebounceFlushReason::SleepOrLock
        );
        assert!(checkpoints.publishes.is_empty());

        debouncer.record_checkpoint_completed("w_main", 1, seconds(1));
        let publishes = debouncer.flush_for_sleep_or_lock(seconds(2));
        assert!(publishes.checkpoints.is_empty());
        assert_eq!(publishes.publishes.len(), 1);
        assert_eq!(
            publishes.publishes[0].reason,
            DebounceFlushReason::SleepOrLock
        );
    }

    #[test]
    fn debounce_checkpoint_paths_are_coalesced_and_sorted() {
        let mut debouncer = AdaptiveDebouncer::new(policy());
        debouncer.observe_change(change("w_main", 1, &["b.txt", "a.txt"]), seconds(0));
        debouncer.observe_change(change("w_main", 2, &["b.txt", "c.txt"]), seconds(1));

        let drain = debouncer.drain_due(seconds(6));

        assert_eq!(drain.checkpoints.len(), 1);
        assert_eq!(drain.checkpoints[0].source_generation, 2);
        assert_eq!(
            drain.checkpoints[0].paths,
            vec![
                PathBuf::from("a.txt"),
                PathBuf::from("b.txt"),
                PathBuf::from("c.txt")
            ]
        );
    }
}
