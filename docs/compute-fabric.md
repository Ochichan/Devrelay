# Compute Fabric

Last updated: 2026-06-24

Compute fabric task execution must not take writer ownership of an active
workspace. The current implementation covers the task definition layer only.

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
