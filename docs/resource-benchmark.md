# Resource Benchmark Plan

Last updated: 2026-07-02

Background protection is product behavior. An always-running personal agent
must be measured before DevRelay claims invisible protection.

## Gate Status

Representative macOS measurements are recorded in
[`output/resource-benchmark-representative-2026-07-02.md`](../output/resource-benchmark-representative-2026-07-02.md):
idle scaling across 0/10/50 registered projects, a dirty-repository burst, and
a 20k-file formatter storm, all inside the numeric budgets below. Linux runs,
battery/anchor scenarios, and watcher-driven bursts remain open.

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

The harness accepts `--project-count` (0 measures an empty idle agent, higher
values register filler projects), `--dirty-files` (tracked files rewritten
before each checkpoint to simulate formatter storms), and `--scenario` for the
report scope line.

Initial macOS smoke evidence is recorded in
[`output/resource-benchmark-results-2026-06-24.md`](../output/resource-benchmark-results-2026-06-24.md).
Representative macOS evidence covering idle scaling, dirty bursts, and a
formatter storm is recorded in
[`output/resource-benchmark-representative-2026-07-02.md`](../output/resource-benchmark-representative-2026-07-02.md).

## Budgets

Numeric budgets, set from the 2026-07-02 representative macOS run (dev-profile
agent; release builds may only improve):

- idle CPU p95 <= 0.5% (measured 0.00% at 0/10/50 registered projects)
- idle RSS <= 32 MiB with up to 50 registered projects (measured 16.36 MiB)
- checkpoint burst RSS peak <= 64 MiB (measured 19.80 MiB in a 20k-file storm)
- checkpoint latency p95 <= 1 s on ~1k-file repos (measured 0.246 s)
- checkpoint latency p95 <= 5 s in 20k-file formatter storms (measured 2.798 s)

Still qualitative until measured:

- battery mode should lengthen debounce and reduce network work
- polling fallback should be explicit in diagnostics
- anchor transfer bytes per hour should be bounded and observable
