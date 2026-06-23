# Glossary

Last updated: 2026-06-23

## Fabric

The user's trusted personal DevRelay network. A fabric contains paired devices,
fabric identity, project metadata, writer leases, snapshots, and audit records.
The first implementation is LAN-first and local-user oriented.

## Device

A DevRelay identity capable of running an agent. Windows native, WSL, macOS,
and Linux are distinct device boundaries even when two of them run on the same
physical hardware. Each WSL distro is treated as a separate boundary.

## Project

A logical Git-backed development unit registered with DevRelay. A project has a
stable project ID, display name, optional manifest, workspace mappings,
snapshot metadata, and per-project storage such as `snapshots.git` and CAS.

## Workspace

A local checkout or recovery directory for a project on a specific device.
Workspaces can be active, inactive, forked, archived, or stale depending on the
lease and session state.

## Session

A line of work within a project. The canonical session has one active writer at
a time. Stale or inactive edits are preserved as separate sessions rather than
merged automatically.

## Snapshot

An immutable checkpoint containing semantic Git state: HEAD, index tree,
working tree overlay, accepted untracked files, sidecar references, excluded
path reasons, and verification hashes. Snapshots are data. They are not writer
authority.

## Lease

The mutable record that identifies which device/workspace may publish the
canonical latest snapshot for a session. Lease transitions are epoch-based and
must be monotonic.

## Handoff

The protocol that moves writer authority from source device to target device.
It checkpoints source work, verifies target application, checks source
generation, and transfers the lease only after the target is verified.

## Anchor

A long-running coordinator/cache role that stores metadata, snapshot bare
repos, CAS chunks, leases, handoffs, and audit records. An anchor lets a target
continue from the latest anchored checkpoint even if the source later sleeps.

## Capsule

An operation-specific state bundle for advanced Git or editor context that
cannot be represented as a normal clean Git snapshot alone. Conflict and
sequencer state are examples.

## Sidecar

Content stored outside normal Git object refs, usually large accepted files or
local-only artifacts. Sidecars are addressed through CAS manifests and verified
by hash before materialization.

## Canonical Latest

The latest snapshot pointer for the active canonical session. Only a valid
lease holder can advance it. Stale, inactive, or partial uploads must preserve
data without changing this pointer.
