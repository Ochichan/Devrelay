# Resource Benchmark Results

Date: 2026-07-02T16:02:36+09:00

Scenario: idle, 0 registered projects. The harness records idle agent CPU/RSS and
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
| Project count | 0 |
| Dirty tracked files per checkpoint | 0 |
| Repository size | 0 KiB |
| Tracked file count | 0 |
| Accepted untracked file count | 0 |
| Sidecar byte count | 0 |

## Idle Agent

| Metric | Value |
| --- | --- |
| Duration | 30.0s |
| Samples | 115 |
| CPU p50 | 0.00% |
| CPU p95 | 0.00% |
| RSS median | 13.47 MiB |
| RSS peak | 13.47 MiB |

## Checkpoint Burst

| Metric | Value |
| --- | --- |
| Iterations | 0 |
| Agent CPU peak | 0.00% |
| Agent RSS peak | 0.00 MiB |
| Checkpoint elapsed p50 | 0.000s |
| Checkpoint elapsed p95 | 0.000s |
| Last snapshot id | none |

## Notes

- Default `cargo test --workspace` currently fails in this environment when the
  configured sccache wrapper compiles Tauri proc macros; verification commands
  use `RUSTC_WRAPPER=` until that wrapper issue is fixed.
- Git status frequency, checkpoints per hour, transfer bytes per hour, watcher
  event counts, sidecar hashing time, and SQLite transaction time still require
  internal instrumentation before they can be treated as release evidence.
- Default wrapper build failure observed before this run: no.
