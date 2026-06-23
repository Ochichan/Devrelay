# ADR 0004: Local Bare Repo Data Plane First

Date: 2026-06-23

## Status

Accepted

## Context

M5 needs Git object transfer without relying on ad hoc source workspace paths.
The roadmap allows the first implementation to use local bare repo fetch/push
endpoints before a network transport is finalized.

## Decision

Use per-project local bare Git repositories as the first data-plane strategy.
Anchor serve handlers expose only an authorized serve plan for a project snapshot
repo, constrained to `refs/devrelay/*` snapshot refs.

## Consequences

- The first data plane keeps Git object transfer on the Git CLI path already
  covered by snapshot and anchor tests.
- The serve plan is a narrow boundary for later mTLS or RPC handlers.
- Network listeners still need to enforce the same authorization, ref namespace,
  object availability, object size, and quota checks before becoming remote
  endpoints.
