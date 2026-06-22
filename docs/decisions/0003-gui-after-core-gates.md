# ADR 0003: GUI After Core Verification Gates

Date: 2026-06-22

## Status

Accepted

## Context

DevRelay's product value depends on safe continuation and recovery. A desktop UI
would be misleading if checkpoint, apply, verification, and dirty-target safety
are not already reliable.

## Decision

Defer production GUI work until the core Git round-trip and local CLI safety
gates are verified.

## Consequences

- M0 and M1 prioritize tests, stable CLI behavior, recovery semantics, and error
  contracts.
- The static UI prototype remains a product reference, not the implementation
  source of truth.
- Desktop work can start with a smaller API surface once the agent owns state.

