# Representative Resource Benchmark Results (macOS)

Date: 2026-07-02

This report aggregates five harness runs covering the resource-benchmark plan
scenarios that the repeatable harness can drive today: idle scaling across
0/10/50 registered projects, a dirty-repository checkpoint burst, and a
formatter-storm checkpoint on a large repository. Raw per-run reports live
next to this file.

## Environment

| Metric | Value |
| --- | --- |
| DevRelay git commit | d5f3e5acebc7b56525a3e14903f597257a656572 |
| OS | Darwin 25.5.0 (macOS) |
| Architecture | arm64 |
| Git version | git version 2.50.1 (Apple Git-155) |
| Filesystem | APFS |
| Power source | AC power |
| Resource profile | default |
| Build profile | dev (unoptimized agent; release numbers can only improve) |
| Idle sampling | 30 s per idle scenario, 0.25 s interval |

## Idle Agent Scaling

| Registered projects | CPU p50 | CPU p95 | RSS median | RSS peak |
| --- | --- | --- | --- | --- |
| 0 | 0.00% | 0.00% | 13.47 MiB | 13.47 MiB |
| 10 | 0.00% | 0.00% | 15.83 MiB | 15.83 MiB |
| 50 | 0.00% | 0.00% | 16.36 MiB | 16.36 MiB |

Idle RSS grows less than 3 MiB from 0 to 50 registered projects and idle CPU
stays at 0% across 115 samples per scenario.

## Checkpoint Bursts

| Scenario | Tracked files | Rewritten per pass | Agent CPU peak | Agent RSS peak | Checkpoint p50 | Checkpoint p95 |
| --- | --- | --- | --- | --- | --- | --- |
| Dirty repository, 10 checkpoints | 1,002 | 1 + untracked note | 8.40% | 16.97 MiB | 0.181 s | 0.246 s |
| Formatter storm, 3 checkpoints | 20,002 | 5,000 | 72.00% | 19.80 MiB | 2.778 s | 2.798 s |

The formatter-storm burst saturates well under one core for under three
seconds per checkpoint and returns to 0% idle afterwards; RSS stays under
20 MiB even while snapshotting 5,000 rewritten tracked files.

## Budgets Met

These measurements replace the provisional qualitative budgets in
`docs/resource-benchmark.md`:

| Budget | Target | Measured | Met |
| --- | --- | --- | --- |
| Idle CPU p95 | <= 0.5% | 0.00% (0/10/50 projects) | yes |
| Idle RSS with up to 50 projects | <= 32 MiB | 16.36 MiB peak | yes |
| Checkpoint burst RSS peak | <= 64 MiB | 19.80 MiB (20k-file storm) | yes |
| Checkpoint latency p95, ~1k-file repo | <= 1 s | 0.246 s | yes |
| Checkpoint latency p95, 20k-file storm | <= 5 s | 2.798 s | yes |

## Not Yet Covered

- Anchor online/offline and transfer bytes per hour: needs an anchor-mode
  harness scenario.
- Battery and low-power debounce behavior: needs battery-power measurement.
- Watcher-driven background checkpoints and event-to-scan coalescing: the
  harness triggers checkpoints through the CLI, not the filesystem watcher.
- Git status frequency, sidecar hashing time, and SQLite transaction time:
  need internal instrumentation.
- Linux and Windows runs of the same matrix.
