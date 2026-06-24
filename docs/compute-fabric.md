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
back through the existing run list APIs. Actual immutable execution snapshot
creation is still open and must be completed before remote task scheduling.
