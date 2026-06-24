# Compute Fabric

Last updated: 2026-06-24

Compute fabric task execution must not take writer ownership of an active
workspace. The current implementation covers task definitions, immutable
execution snapshots, task run metadata, scheduler constraint filtering, and
explainable scheduler scoring, isolated runner workspace preparation, and host
task execution, log/artifact storage, result cache metadata, and Nix delegation
planning.

## Task Model

Task definitions come from `devrelay.toml` and must reference an existing
environment profile. The manifest validator rejects empty commands, empty
platform/output/feature entries, invalid resource hints, and invalid sandbox
values.

The core task model normalizes each task into a `TaskDefinition` with a
task-specific command definition hash. The hash includes the task command,
platform constraints, resource hints, cache/output/feature/sandbox settings,
and the selected environment profile definition.

Task run metadata can be recorded in the per-project SQLite database and read
back through the existing run list APIs.

## Execution Snapshots

A task execution snapshot stores the current Git state as a pinned snapshot and
binds it to the normalized task definition hash. The source workspace refs are
removed after import, while the per-project snapshot store keeps the immutable
refs and metadata for later scheduling or audit.

Applying execution snapshots to remote workers remains open.

## Scheduler Constraints

Scheduler constraint filtering starts with a device resource snapshot:

- static OS, architecture, CPU cores, memory capacity, disk capacity, and
  platform capability-derived features;
- dynamic CPU load, free memory, free disk, power state, low-power mode,
  foreground load, and an explicit network route quality placeholder;
- local policy that can disallow task execution before scoring.

The filter rejects incompatible task platform globs, missing task features,
insufficient CPU, memory, or disk, unknown required resource metrics, and devices
paused by local policy. Later M10 scheduler scoring can rank only the eligible
candidate set.

## Scheduler Scoring

Scheduler scoring runs after constraints. Ineligible devices receive score `0`
with constraint rejection details. Eligible devices receive an explainable
0..1000 score from weighted component signals:

- cache warmth, idle CPU, free memory, power preference, data locality, network
  quality, historical speed, and user affinity;
- transfer cost, foreground activity, and thermal pressure penalty-style scores;
- task-class weight profiles for interactive, test, build, batch, and
  background tasks.

Unknown signals are neutral rather than silently treated as ideal. The score
output includes every component, its normalized score, its weight, weighted
points, and a short explanation string.

## Runner Workspace

Runner workspace preparation creates a disposable Git repository under the
project's DevRelay data directory, applies the immutable task execution snapshot
from the snapshot store, and marks the workspace as non-canonical so it cannot
be treated as a writer session. The task's declared environment profile is
validated against the runner platform and available environment kind before the
workspace is returned.

Sidecars are materialized through CAS when the snapshot requires them. Required
secrets are materialized only when the caller provides explicit permission,
local mappings, and a provider; otherwise the workspace records skipped required
secret names without writing secret files. Cleanup follows the runner workspace
retention policy, with delete-on-cleanup as the default.

## Runner Execution

Host task execution runs the task command inside the prepared runner workspace.
It resolves the working directory inside that workspace, combines explicit
environment variables with permitted secret environment variables, applies the
task timeout, captures exit code/stdout/stderr, and forwards stdout/stderr chunks
to a live log sink while retaining the same chunks on the execution result.

Timeouts cancel the process tree on Unix by starting the command in its own
process group and killing that group. Sandbox, container, and VM execution modes
are explicit placeholders that fail closed rather than silently running on the
host.

## Logs

Task log storage implements the execution log sink. It keeps a bounded in-memory
live buffer for recent stdout/stderr chunks and writes redacted JSONL records to
a per-run disk spool under the project data directory. Retrieval reads the spool
back as structured records and reports whether a truncation marker was emitted.

The spool fails closed on unsafe task run IDs and stops appending regular chunks
after its byte limit is reached, preserving a final truncation record.

## Artifacts

Artifact capture reads the task definition's declared output globs, rejects
absolute or escaping paths, walks the runner workspace, and uploads matched files
to the project CAS. Each captured artifact stores its path, size, BLAKE3 chunk
hash, CAS manifest ID, and reachability root in a per-run artifact index.

The capture API returns a summary first: count, missing output patterns, total
bytes, index path, and the full index. Artifacts can be pulled on demand from
CAS into a destination root, and artifact retention removes the per-artifact CAS
reachability roots.

## Result Cache

Result cache keys are derived from deterministic task inputs: input snapshot
state and tree OIDs, declared sidecar inputs, environment fingerprint, command
definition hash, platform key, and declared outputs. The key builder does not
accept secret values.

Cache eligibility follows the task cache mode (`off`, `read`, `write`,
`read-write`). Manifests that declare secrets are treated as secret-sensitive
and disabled by default unless the caller explicitly opts in. Cache entries are
stored under the project data directory as metadata pointing at the per-run
artifact index. Lookups return a structured hit only when the entry and artifact
index still match the requested key, and hits can restore cached artifacts from
CAS into a destination root.

## Nix Delegation

Nix delegation planning detects tasks whose environment profile kind is `nix`.
Before delegating, it reuses scheduler constraint evaluation for platform,
feature, CPU, memory, disk, and policy checks, then requires the Nix adapter to
report an available command and ready shell.

Eligible plans generate a temporary builder set under `.devrelay/nix`, preserve
the task command and declared outputs, and produce command plans for
`nix build --print-build-logs` so remote builder logs can be streamed. When a
LAN binary cache target is configured, the plan also emits a `nix copy --to`
command for publishing the result. Rejected plans carry explicit explanations
for non-Nix tasks, device constraint failures, unavailable Nix, or failed Nix
health checks.

Remote execution dispatch remains later M10 work.
