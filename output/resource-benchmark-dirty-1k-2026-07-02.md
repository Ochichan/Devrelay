# Resource Benchmark Results

Date: 2026-07-02T16:04:11+09:00

Scenario: dirty checkpoint burst, 1k tracked files. The harness records idle agent CPU/RSS and
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
| Dirty tracked files per checkpoint | 0 |
| Repository size | 8404 KiB |
| Tracked file count | 1002 |
| Accepted untracked file count | 1 |
| Sidecar byte count | 0 |

## Idle Agent

| Metric | Value |
| --- | --- |
| Duration | 10.0s |
| Samples | 39 |
| CPU p50 | 0.00% |
| CPU p95 | 0.10% |
| RSS median | 15.23 MiB |
| RSS peak | 15.23 MiB |

## Checkpoint Burst

| Metric | Value |
| --- | --- |
| Iterations | 10 |
| Agent CPU peak | 8.40% |
| Agent RSS peak | 16.97 MiB |
| Checkpoint elapsed p50 | 0.181s |
| Checkpoint elapsed p95 | 0.246s |
| Last snapshot id | s1_62d74d7308f3fb9071193977 |

## Notes

- Default `cargo test --workspace` currently fails in this environment when the
  configured sccache wrapper compiles Tauri proc macros; verification commands
  use `RUSTC_WRAPPER=` until that wrapper issue is fixed.
- Git status frequency, checkpoints per hour, transfer bytes per hour, watcher
  event counts, sidecar hashing time, and SQLite transaction time still require
  internal instrumentation before they can be treated as release evidence.
- Default wrapper build failure observed before this run: no.
