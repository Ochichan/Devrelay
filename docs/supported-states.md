# Supported States

Last updated: 2026-06-23

This document describes the states the current implementation intends to
preserve or safely block. A supported state still requires passing its
verification gate before it can be claimed in product UX.

## Supported For Local Round Trip

- Clean repository.
- Staged add, modify, and delete.
- Unstaged modify and delete.
- Staged and unstaged combinations that Git can represent with a merged index.
- Accepted untracked files under the configured untracked policy.
- Excluded generated directories and secret-like paths.
- Binary files.
- Empty files.
- Unicode paths.
- Paths with spaces.
- Renames reported by Git porcelain v2.
- POSIX executable bit where the target platform can verify it.
- Dirty target detection before apply.

## Supported For Local Recovery

- Snapshot list/show/export from the local store.
- Recovery open into a new or clean workspace.
- Dirty recovery target refusal.
- Dirty target backup through `snapshot-and-fork`.
- Dirty target preservation through `new-workspace`.

## Supported For Lease And Handoff Metadata

- Active writer lease.
- Stale publish rejection.
- Inactive publish preservation as a fork.
- Concurrent handoff rejection.
- Handoff target verification before lease commit.
- Handoff crash recovery before and after lease commit.
- Expired handoff abort.

## Supported For Data Plane

- Per-project bare snapshot repositories.
- Authorized Git data-plane serve plans limited to `refs/devrelay/*`.
- Anchor snapshot cache.
- Direct route and anchor fallback route selection.
- CAS chunk upload/download with hash verification.
- Large sidecar capture and bounded materialization.
- Partial upload safety before metadata publish.

## Supported For Background Protection

- Filesystem events as hints only.
- Polling fallback for unsupported watcher platforms.
- Adaptive debounce.
- Semantic no-op checkpoint skipping.
- Resource policy persistence.
- Retention and quota planning.
- Crash journal replay for snapshot, publish, target apply, and lease phases.

Resource budgets are not yet proven. Do not claim invisible protection until
the benchmark plan in [resource-benchmark.md](resource-benchmark.md) has
results.

## Supported For Cross-Platform Doctors

- Platform identity for Linux, macOS, Windows native, and WSL.
- Case-fold and Unicode normalization collision detection.
- Windows reserved name and invalid character detection.
- Path length budget warnings.
- Line-ending policy warnings and Git semantic verification.
- Symlink and reparse traversal defenses.
- WSL/native shared-tree warning.

## Supported For Advanced Git States

- Merge, cherry-pick, and revert conflict capture.
- Index stage 1/2/3 preservation.
- Conflict worktree file restoration.
- Clean submodule recorded commit restore.
- Dirty submodule preservation as child project/session metadata.
- LFS pointer detection and local-only LFS object sidecar fallback.
- Sparse checkout and partial clone detection.
- Interactive rebase/sequencer detection and safe block fallback.

Interactive rebase reconstruction is intentionally disabled until exhaustive
tests exist.
