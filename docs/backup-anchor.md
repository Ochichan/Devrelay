# Backup Anchor Data Set

Last updated: 2026-06-24

Backup anchor support is intended to recover from loss of the primary anchor
without letting backup state advance canonical work by itself. This document
defines the data set that must be replicated before implementation work begins.

## Recovery Objective

A backup anchor should be able to restore:

- latest known project/session metadata,
- published immutable snapshots,
- snapshot Git objects,
- CAS sidecars required by those snapshots,
- lease/handoff/audit history needed to explain state,
- revocation state needed to reject stale devices.

It must not become an independent writer. Promotion to primary anchor must be a
manual, audited operation.

## Required Data Set

### Metadata DB

Replicate a consistent SQLite backup of:

- projects and sessions
- devices, public fabric identity, revocations
- leases and handoff records
- stored snapshot metadata
- task run records needed for activity history
- command trust approvals
- audit events
- schema migration state

The backup must be taken with SQLite's backup API or an equivalent consistent
WAL-aware snapshot, never by copying a live main DB file alone.

### Snapshot Bare Repositories

Replicate per-project anchor snapshot bare repos, including:

- `refs/devrelay/snapshots/*`
- required Git objects reachable from published snapshot refs
- repository config needed to serve read-only snapshot fetches

Excluded:

- arbitrary non-DevRelay refs
- reflogs that are not required for recovery
- temporary import/export refs

### CAS Roots And Chunks

Replicate CAS data reachable from stored snapshot metadata and sidecar
manifests:

- sidecar reachability roots
- chunk records
- chunk files
- manifest records

Unreachable cache/task artifacts may be excluded unless a later release defines
them as recovery-critical.

### Trust And Audit State

Replicate state required to preserve security decisions:

- device revocations
- command trust approvals and rejections
- audit events for publish, handoff, revoke, command trust, cleanup, and future
  backup promotion

Audit history is recovery evidence and must not be silently truncated by backup
promotion.

## Consistency Rules

- Metadata backup defines the manifest of required Git refs and CAS roots.
- Git/CAS replication must complete before the backup is considered restorable.
- A backup generation should include a signed manifest over metadata backup ID,
  Git ref tips, CAS reachability roots, byte counts, and creation time.
- Backup generations are append-only until retention removes a complete old
  generation.
- Partial backup generations are not promotion candidates.

## Promotion Rules

Promotion is manual and must require:

- operator confirmation naming source and target anchors,
- verification that metadata, snapshot refs, and CAS roots match the signed
  backup manifest,
- revocation state freshness check,
- audit event recording the promotion,
- new anchor identity advertisement only after verification succeeds.

Promotion must not transfer writer leases by itself. Devices still need normal
lease and handoff checks after reconnecting to the promoted anchor.

## Non-Goals

- Automatic split-brain resolution.
- Backup anchor as an always-writable secondary.
- Restoring local uncommitted work that was never checkpointed or anchored.
- Opaque/encrypted anchor storage; see `docs/opaque-anchor-research.md`.
