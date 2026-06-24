# Resource Benchmark Results

Date: 2026-06-24T17:45:06+09:00

Scope: initial macOS smoke run for the benchmark harness. These numbers prove
the harness can record idle agent CPU/RSS and checkpoint burst behavior; they
are not release budgets.

## Environment

| Metric | Value |
| --- | --- |
| DevRelay git commit | d9d2b5e1377d6e192ab44d899f95005bf6088d21 |
| Worktree dirty during measurement | yes |
| OS | Darwin 25.5.0 |
| Architecture | arm64 |
| Git version | git version 2.50.1 (Apple Git-155) |
| Filesystem type | APFS |
| Power source | Now drawing from 'AC Power' |
| Resource profile | default |
| Watcher backend | agent foreground / no watcher workload |
| Project count | 1 |
| Repository size | 1024 KiB |
| Tracked file count | 102 |
| Accepted untracked file count | 1 |
| Sidecar byte count | 0 |

## Idle Agent

| Metric | Value |
| --- | --- |
| Duration | 5.0s |
| Samples | 20 |
| CPU p50 | 0.00% |
| CPU p95 | 0.40% |
| RSS median | 14.31 MiB |
| RSS peak | 14.31 MiB |

## Checkpoint Burst

| Metric | Value |
| --- | --- |
| Iterations | 5 |
| Agent CPU peak | 3.50% |
| Agent RSS peak | 15.34 MiB |
| Checkpoint elapsed p50 | 0.146s |
| Checkpoint elapsed p95 | 0.201s |
| Last snapshot id | s1_6d06ea76a90d04e4acdcbeb6 |

## Notes

- Default `cargo test --workspace` currently fails in this environment when the
  configured sccache wrapper compiles Tauri proc macros; verification commands
  use `RUSTC_WRAPPER=` until that wrapper issue is fixed.
- Git status frequency, checkpoints per hour, transfer bytes per hour, watcher
  event counts, sidecar hashing time, and SQLite transaction time still require
  internal instrumentation before they can be treated as release evidence.
- Default wrapper build failure observed before this run: yes.
