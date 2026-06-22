# ADR 0001: Rust Core And Git CLI Orchestration

Date: 2026-06-22

## Status

Accepted

## Context

DevRelay's first correctness gate is a verified local Git round trip. The core
must reason about HEAD, index, work tree state, untracked policy, and snapshot
verification before higher-level agent or UI work is trusted.

## Decision

Build the core in Rust and orchestrate Git through the installed Git CLI for the
M0 foundation.

## Consequences

- Rust gives the core explicit data types, predictable error handling, and a
  clear path to a long-running local agent.
- The Git CLI keeps M0 aligned with Git's supported porcelain and plumbing
  behavior without embedding a partial Git implementation.
- Git command boundaries must stay typed and tested so future storage changes
  can replace internals without changing the product contract.

