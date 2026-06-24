# Resource Benchmark Plan

Last updated: 2026-06-24

Background protection is product behavior. An always-running personal agent
must be measured before DevRelay claims invisible protection.

## Open Gate

M6 is not complete until idle CPU/RSS and checkpoint burst measurements are
recorded on representative repositories.

## Scenarios

Measure at least:

- registered projects: 0
- registered projects: 10
- registered projects: 50
- clean repository
- dirty repository
- large monorepo
- formatter touching thousands of files
- anchor online
- anchor offline
- battery mode
- polling fallback mode

## Metrics

Record:

- idle CPU p50
- idle CPU p95
- idle RSS
- checkpoint CPU burst
- checkpoint peak RSS
- Git status call frequency
- checkpoints per hour
- transfer bytes per hour
- watch events received
- actual scans performed
- event-to-scan coalescing ratio
- time spent hashing sidecars
- time spent in SQLite transactions

## Environment Metadata

Each run should include:

- DevRelay git commit
- OS and version
- architecture
- Git version
- filesystem type if known
- power source
- resource profile
- watcher backend
- project count
- repository size
- tracked file count
- accepted untracked file count
- sidecar byte count

## Suggested Output

Write results as Markdown under:

```text
target/resource-benchmarks/<date>-<platform>.md
```

Before beta, copy the accepted summary into a tracked document or release gate
report. Raw target output does not need to be committed.

## Harness

Run the repeatable local harness with:

```bash
just resource-benchmark
```

The harness builds the CLI and agent, starts a foreground agent with an isolated
`DEVRELAY_HOME`, samples agent CPU/RSS while idle, creates a temporary Git
project, registers it through the agent socket, and samples the agent during a
checkpoint burst.

Use a tracked output path when a result should become project evidence:

```bash
python3 scripts/resource_benchmark.py \
  --out output/resource-benchmark-results-2026-06-24.md \
  --idle-seconds 5 \
  --checkpoint-iterations 5 \
  --tracked-files 100
```

Initial macOS smoke evidence is recorded in
[`output/resource-benchmark-results-2026-06-24.md`](../output/resource-benchmark-results-2026-06-24.md).
It records idle CPU/RSS and checkpoint CPU/RSS burst behavior, but it is not a
representative release benchmark.

## Initial Budgets

Budgets are provisional until measured:

- idle CPU p95 should be low enough to disappear under normal desktop load
- idle RSS should remain stable with 0, 10, and 50 registered projects
- formatter storms should coalesce into bounded scans and checkpoints
- battery mode should lengthen debounce and reduce network work
- polling fallback should be explicit in diagnostics

The benchmark report should replace these qualitative statements with numeric
budgets once the harness exists.
