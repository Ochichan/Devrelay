# Data-Loss Safety Policy

Last updated: 2026-06-23

DevRelay is allowed to be conservative, noisy, or slower when safety is at
risk. It is not allowed to silently discard user work.

## Non-Negotiable Rules

- No implementation path can silently overwrite target work.
- No background path performs an automatic merge.
- No plaintext secret is included in a snapshot by default.
- No remote command runs without trust hash approval.
- No UI computes canonical state independently from the agent.
- No watcher event is treated as the source of truth.
- No cross-device handoff succeeds before verification passes.
- No compute task writes directly into the active session.
- Every destructive cleanup has explicit confirmation or a prior snapshot.
- Every recovery operation defaults to a new session or workspace.
- Every published snapshot is immutable.
- Every lease epoch transition is monotonic.
- Every stale publish preserves data as non-canonical work.
- Every diagnostic export is redacted by default.

## Dirty Target Policy

The default dirty policy is `block`.

If a user chooses to continue anyway, the only allowed safe actions are:

- `snapshot-and-fork`: capture target work as a pinned fork before applying
  incoming work.
- `new-workspace`: leave the dirty target unchanged and open incoming work in a
  separate workspace.
- `cancel`: make no changes.

The UI must not expose `snapshot-and-fork` as internal jargon. User-facing copy
should say "Preserve it as separate work and continue".

## Inactive Edit Policy

An inactive workspace can create useful work, but it cannot advance canonical
latest. DevRelay must preserve inactive edits as a fork session, pin the fork
snapshot by default, and leave canonical latest unchanged.

## Handoff Policy

A handoff may transfer lease ownership only after:

1. Source work is checkpointed.
2. Required Git objects and sidecars are available.
3. Target apply completes.
4. Target verification passes.
5. Source generation still matches the handoff expectation.
6. The lease commit succeeds in one metadata transaction.

Failure before step 6 must leave the previous writer in control or recover to a
clear committed state. Missing chunks, dirty targets, source edits, expired
handoffs, and verification mismatches are blockers.

## Watcher Policy

Filesystem watcher events are hints. They may trigger a scan, increment a
generation, or schedule a checkpoint. They may not directly become canonical
state.

## Cleanup Policy

Retention and quota pruning may remove only evictable data. Latest canonical,
pinned snapshots, handoff-protected snapshots, and recovery-critical data are
protected.

## Evidence Mapping

The final safety checklist must be tied to integration suite names:

| Invariant | Required suite |
| --- | --- |
| No dirty target silent overwrite | `safety/no_silent_overwrite` |
| No lease transfer before verification | `safety/no_unverified_handoff` |
| Stale/inactive publish does not advance latest | `safety/stale_publish_is_fork` |
| Secret-like files stay out of snapshots by default | `safety/no_plaintext_secret_snapshot` |
| Remote commands require trust hash approval | `safety/no_untrusted_remote_execution` |
| UI state comes only from agent events/RPC | `safety/ui_has_no_state_authority` |
| Watcher events are hints only | `safety/watcher_events_are_hints` |
| Remote tasks do not mutate active sessions | `safety/no_active_workspace_remote_task` |
| Destructive dirty-target cleanup requires a recoverable pinned backup | `safety/destructive_cleanup_has_snapshot` |
| Diagnostic exports are redacted by default | `safety/diagnostics_redacted_by_default` |

Existing unit and integration tests cover many underlying cases. The suite
names above are still the required product-level evidence before beta.
