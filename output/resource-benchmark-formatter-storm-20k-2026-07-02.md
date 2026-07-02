# Resource Benchmark Results

Date: 2026-07-02T16:04:41+09:00

Scenario: formatter storm, 20k tracked files, 5k rewritten per checkpoint. The harness records idle agent CPU/RSS and
checkpoint burst behavior for this configuration.

## Environment

| Metric | Value |
| --- | --- |
| DevRelay git commit | d5f3e5acebc7b56525a3e14903f597257a656572 |
| Worktree dirty during measurement | yes |
| OS | Darwin 25.5.0 |
| Architecture | arm64 |
| Git version | git version 2.50.1 (Apple Git-155) |
| Filesystem type | APFS |
| Power source | Now drawing from 'AC Power' |
| Resource profile | default |
| Watcher backend | agent foreground / no watcher workload |
| Project count | 1 |
| Dirty tracked files per checkpoint | 5000 |
| Repository size | 144932 KiB |
| Tracked file count | 20002 |
| Accepted untracked file count | 1 |
| Sidecar byte count | 0 |

## Idle Agent

| Metric | Value |
| --- | --- |
| Duration | 10.0s |
| Samples | 39 |
| CPU p50 | 0.00% |
| CPU p95 | 0.30% |
| RSS median | 15.27 MiB |
| RSS peak | 15.27 MiB |

## Checkpoint Burst

| Metric | Value |
| --- | --- |
| Iterations | 3 |
| Agent CPU peak | 72.00% |
| Agent RSS peak | 19.80 MiB |
| Checkpoint elapsed p50 | 2.778s |
| Checkpoint elapsed p95 | 2.798s |
| Last snapshot id | s1_c9729a2cde1d76f1756fe38a |

## Notes

- Default `cargo test --workspace` currently fails in this environment when the
  configured sccache wrapper compiles Tauri proc macros; verification commands
  use `RUSTC_WRAPPER=` until that wrapper issue is fixed.
- Git status frequency, checkpoints per hour, transfer bytes per hour, watcher
  event counts, sidecar hashing time, and SQLite transaction time still require
  internal instrumentation before they can be treated as release evidence.
- Default wrapper build failure observed before this run: no.
