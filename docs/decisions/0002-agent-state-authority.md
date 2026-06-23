# ADR 0002: Agent As UI State Authority

Date: 2026-06-22

## Status

Accepted

## Context

DevRelay will eventually expose a CLI, desktop UI, tray controls, and editor
surfaces. These surfaces can become inconsistent if each one tracks workspace,
snapshot, lease, and recovery state independently.

## Decision

The local agent is the only authority for user-visible DevRelay state. UI
surfaces request state, commands, and events from the agent instead of deriving
their own truth from the filesystem or Git.

## Consequences

- UI clients remain thin and recoverable.
- The agent API must be stable before desktop and editor UX become primary
  surfaces.
- CLI-only M0 work should avoid embedding assumptions that would later compete
  with the agent.
- Production UI must not read Git directly, scan registered workspaces directly,
  infer lease holder locally, or treat watcher events as canonical state.
- Windows UI support depends on Windows named pipe IPC and per-user pipe ACL
  because the UI cannot bypass the agent on Windows.
