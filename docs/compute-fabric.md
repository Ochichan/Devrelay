# Compute Fabric

Last updated: 2026-06-24

Compute fabric task execution must not take writer ownership of an active
workspace. The current implementation covers task definitions, immutable
execution snapshots, task run metadata, scheduler constraint filtering, and
explainable scheduler scoring.

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

Applying execution snapshots to remote workers, collecting logs/artifacts, and
cache reuse are still open.

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
