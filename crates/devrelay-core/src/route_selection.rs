//! Snapshot transfer route measurement and selection.
//!
//! Route selection remains pure once measurements are available. Local probes
//! can fill those measurements from source and anchor repositories before the
//! selector returns the route, fallback, explanation, and metrics that callers
//! can log or expose.

use crate::{AnchorSnapshotRepo, GitRepo, SnapshotMetadata};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SnapshotTransferRoute {
    DirectPeer,
    AnchorCache,
    SourceRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRouteMeasurements {
    pub source_online: bool,
    pub anchor_available: bool,
    pub anchor_has_snapshot: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct SnapshotRouteMeasurementInput<'a> {
    pub source_repo: Option<&'a GitRepo>,
    pub anchor_repo: Option<&'a AnchorSnapshotRepo>,
    pub snapshot: &'a SnapshotMetadata,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRoutePolicy {
    pub direct_peer_enabled: bool,
    pub anchor_cache_enabled: bool,
    pub require_source: bool,
}

impl Default for SnapshotRoutePolicy {
    fn default() -> Self {
        Self {
            direct_peer_enabled: true,
            anchor_cache_enabled: true,
            require_source: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRouteMetrics {
    pub source_online: bool,
    pub anchor_available: bool,
    pub anchor_has_snapshot: bool,
    pub direct_peer_enabled: bool,
    pub anchor_cache_enabled: bool,
    pub require_source: bool,
    pub fallback_candidate: Option<SnapshotTransferRoute>,
    pub failed_route: Option<SnapshotTransferRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRouteDecision {
    pub route: SnapshotTransferRoute,
    pub available: bool,
    pub explanation: String,
    pub fallback_route: Option<SnapshotTransferRoute>,
    pub metrics: SnapshotRouteMetrics,
}

pub fn measure_snapshot_route(
    input: SnapshotRouteMeasurementInput<'_>,
) -> SnapshotRouteMeasurements {
    let source_online = input
        .source_repo
        .is_some_and(|repo| repo.run(&["rev-parse", "--git-dir"]).is_ok());
    let anchor_available = input.anchor_repo.is_some_and(|anchor| {
        anchor.repo_path().join("HEAD").exists()
            && GitRepo::new(anchor.repo_path())
                .run(&["rev-parse", "--git-dir"])
                .is_ok()
    });
    let anchor_has_snapshot = input
        .anchor_repo
        .is_some_and(|anchor| anchor.verify_snapshot_available(input.snapshot).is_ok());

    SnapshotRouteMeasurements {
        source_online,
        anchor_available,
        anchor_has_snapshot,
    }
}

pub fn select_snapshot_route(
    measurements: SnapshotRouteMeasurements,
    policy: SnapshotRoutePolicy,
) -> SnapshotRouteDecision {
    select_snapshot_route_inner(measurements, policy, None)
}

pub fn select_snapshot_route_after_failure(
    measurements: SnapshotRouteMeasurements,
    policy: SnapshotRoutePolicy,
    failed_route: SnapshotTransferRoute,
) -> SnapshotRouteDecision {
    if !policy.require_source {
        if failed_route == SnapshotTransferRoute::DirectPeer
            && anchor_cache_ready(measurements, policy)
        {
            return decision(
                SnapshotTransferRoute::AnchorCache,
                true,
                "direct peer route failed; falling back to anchor cache",
                None,
                measurements,
                policy,
                Some(failed_route),
            );
        }
        if failed_route == SnapshotTransferRoute::AnchorCache
            && direct_peer_ready(measurements, policy)
        {
            return decision(
                SnapshotTransferRoute::DirectPeer,
                true,
                "anchor cache route failed; falling back to direct peer",
                None,
                measurements,
                policy,
                Some(failed_route),
            );
        }
    }

    decision(
        SnapshotTransferRoute::SourceRequired,
        false,
        "source is required because the selected route failed and no fallback is available",
        None,
        measurements,
        policy,
        Some(failed_route),
    )
}

fn select_snapshot_route_inner(
    measurements: SnapshotRouteMeasurements,
    policy: SnapshotRoutePolicy,
    failed_route: Option<SnapshotTransferRoute>,
) -> SnapshotRouteDecision {
    if policy.require_source {
        return decision(
            SnapshotTransferRoute::SourceRequired,
            direct_peer_ready(measurements, policy),
            "source-required policy selected; cached anchor data will not be used",
            None,
            measurements,
            policy,
            failed_route,
        );
    }

    if direct_peer_ready(measurements, policy) {
        let fallback_route =
            anchor_cache_ready(measurements, policy).then_some(SnapshotTransferRoute::AnchorCache);
        return decision(
            SnapshotTransferRoute::DirectPeer,
            true,
            "source is online; direct peer route selected",
            fallback_route,
            measurements,
            policy,
            failed_route,
        );
    }

    if anchor_cache_ready(measurements, policy) {
        return decision(
            SnapshotTransferRoute::AnchorCache,
            true,
            "source is offline; anchor cache has the snapshot",
            None,
            measurements,
            policy,
            failed_route,
        );
    }

    decision(
        SnapshotTransferRoute::SourceRequired,
        false,
        "source is required because no usable cached snapshot is available",
        None,
        measurements,
        policy,
        failed_route,
    )
}

fn decision(
    route: SnapshotTransferRoute,
    available: bool,
    explanation: &str,
    fallback_route: Option<SnapshotTransferRoute>,
    measurements: SnapshotRouteMeasurements,
    policy: SnapshotRoutePolicy,
    failed_route: Option<SnapshotTransferRoute>,
) -> SnapshotRouteDecision {
    SnapshotRouteDecision {
        route,
        available,
        explanation: explanation.to_string(),
        fallback_route,
        metrics: SnapshotRouteMetrics {
            source_online: measurements.source_online,
            anchor_available: measurements.anchor_available,
            anchor_has_snapshot: measurements.anchor_has_snapshot,
            direct_peer_enabled: policy.direct_peer_enabled,
            anchor_cache_enabled: policy.anchor_cache_enabled,
            require_source: policy.require_source,
            fallback_candidate: fallback_route,
            failed_route,
        },
    }
}

fn direct_peer_ready(measurements: SnapshotRouteMeasurements, policy: SnapshotRoutePolicy) -> bool {
    policy.direct_peer_enabled && measurements.source_online
}

fn anchor_cache_ready(
    measurements: SnapshotRouteMeasurements,
    policy: SnapshotRoutePolicy,
) -> bool {
    policy.anchor_cache_enabled && measurements.anchor_available && measurements.anchor_has_snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DevRelayHome, Manifest, SnapshotStore};
    use std::fs;
    use std::path::Path;

    fn measurements(
        source_online: bool,
        anchor_available: bool,
        anchor_has_snapshot: bool,
    ) -> SnapshotRouteMeasurements {
        SnapshotRouteMeasurements {
            source_online,
            anchor_available,
            anchor_has_snapshot,
        }
    }

    fn manifest() -> Manifest {
        Manifest::parse(
            r#"
schema = 1
project_id = "route-project"
name = "Route Project"

[workspace]
untracked = "safe"
portable_paths = "strict"
"#,
        )
        .unwrap()
    }

    fn init_repo(path: &Path) -> GitRepo {
        fs::create_dir(path).unwrap();
        let repo = GitRepo::new(path);
        repo.run(&["init", "-b", "main"]).unwrap();
        repo.run(&["config", "user.name", "DevRelay Test"]).unwrap();
        repo.run(&["config", "user.email", "devrelay-test@example.local"])
            .unwrap();
        fs::write(path.join("tracked.txt"), "base\n").unwrap();
        repo.run(&["add", "."]).unwrap();
        repo.run(&["commit", "-m", "base"]).unwrap();
        repo
    }

    #[test]
    fn route_measurement_detects_source_and_anchor_availability() {
        let temp = tempfile::tempdir().unwrap();
        let home = DevRelayHome::new(temp.path().join("home"));
        let source_path = temp.path().join("source");
        let source = init_repo(&source_path);
        let manifest = manifest();
        let mut store = SnapshotStore::open(&home, &manifest.project_id).unwrap();

        fs::write(source_path.join("tracked.txt"), "changed\n").unwrap();
        let stored = store.checkpoint(&source, &manifest, false, None).unwrap();
        let anchor = AnchorSnapshotRepo::open(&home, &manifest.project_id).unwrap();
        anchor
            .import_snapshot_from_store(&store, &stored.snapshot_id)
            .unwrap();

        let online = measure_snapshot_route(SnapshotRouteMeasurementInput {
            source_repo: Some(&source),
            anchor_repo: Some(&anchor),
            snapshot: &stored.metadata,
        });
        assert_eq!(online, measurements(true, true, true));

        fs::remove_dir_all(&source_path).unwrap();
        let source_offline = measure_snapshot_route(SnapshotRouteMeasurementInput {
            source_repo: Some(&source),
            anchor_repo: Some(&anchor),
            snapshot: &stored.metadata,
        });
        assert_eq!(source_offline, measurements(false, true, true));

        GitRepo::new(anchor.repo_path())
            .run(&["update-ref", "-d", &stored.metadata.work_ref()])
            .unwrap();
        let missing_anchor_snapshot = measure_snapshot_route(SnapshotRouteMeasurementInput {
            source_repo: None,
            anchor_repo: Some(&anchor),
            snapshot: &stored.metadata,
        });
        assert_eq!(missing_anchor_snapshot, measurements(false, true, false));
    }

    #[test]
    fn direct_route_prefers_online_source_and_reports_anchor_fallback() {
        let decision = select_snapshot_route(
            measurements(true, true, true),
            SnapshotRoutePolicy::default(),
        );

        assert_eq!(decision.route, SnapshotTransferRoute::DirectPeer);
        assert!(decision.available);
        assert_eq!(
            decision.fallback_route,
            Some(SnapshotTransferRoute::AnchorCache)
        );
        assert!(decision.explanation.contains("direct peer"));
        assert_eq!(
            decision.metrics.fallback_candidate,
            Some(SnapshotTransferRoute::AnchorCache)
        );
    }

    #[test]
    fn anchor_cache_route_handles_offline_source() {
        let decision = select_snapshot_route(
            measurements(false, true, true),
            SnapshotRoutePolicy::default(),
        );

        assert_eq!(decision.route, SnapshotTransferRoute::AnchorCache);
        assert!(decision.available);
        assert_eq!(decision.fallback_route, None);
        assert!(decision.explanation.contains("anchor cache"));
    }

    #[test]
    fn source_required_when_no_cached_snapshot_is_available() {
        let decision = select_snapshot_route(
            measurements(false, true, false),
            SnapshotRoutePolicy::default(),
        );

        assert_eq!(decision.route, SnapshotTransferRoute::SourceRequired);
        assert!(!decision.available);
        assert!(decision.explanation.contains("source is required"));
    }

    #[test]
    fn source_required_policy_disables_anchor_cache() {
        let decision = select_snapshot_route(
            measurements(false, true, true),
            SnapshotRoutePolicy {
                require_source: true,
                ..SnapshotRoutePolicy::default()
            },
        );

        assert_eq!(decision.route, SnapshotTransferRoute::SourceRequired);
        assert!(!decision.available);
        assert!(decision.metrics.require_source);
    }

    #[test]
    fn route_failure_falls_back_from_direct_to_anchor_cache() {
        let decision = select_snapshot_route_after_failure(
            measurements(true, true, true),
            SnapshotRoutePolicy::default(),
            SnapshotTransferRoute::DirectPeer,
        );

        assert_eq!(decision.route, SnapshotTransferRoute::AnchorCache);
        assert!(decision.available);
        assert_eq!(
            decision.metrics.failed_route,
            Some(SnapshotTransferRoute::DirectPeer)
        );
        assert!(decision.explanation.contains("falling back"));
    }

    #[test]
    fn route_failure_reports_source_required_without_fallback() {
        let decision = select_snapshot_route_after_failure(
            measurements(false, false, false),
            SnapshotRoutePolicy::default(),
            SnapshotTransferRoute::AnchorCache,
        );

        assert_eq!(decision.route, SnapshotTransferRoute::SourceRequired);
        assert!(!decision.available);
        assert_eq!(
            decision.metrics.failed_route,
            Some(SnapshotTransferRoute::AnchorCache)
        );
    }
}
