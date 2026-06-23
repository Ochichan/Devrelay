//! Snapshot transfer route selection.
//!
//! Route selection is intentionally pure: discovery and network layers provide
//! measurements, and this module returns the route, fallback, explanation, and
//! metrics that callers can log or expose.

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
