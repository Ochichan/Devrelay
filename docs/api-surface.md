# DevRelay Core API Surface

Last reviewed: 2026-06-22

This note records the M0 public API decision for `devrelay-core`.

## Stable For M0

The stable M0 surface is the crate-root API used by `devrelay-cli`:

- manifest loading and typed manifest schema values
- `GitRepo` status collection
- untracked classification values
- snapshot create, read, write, apply, and verify helpers
- `DevRelayError` and `Result`

The manifest module remains public because the manifest schema is part of the
project contract and downstream tools may need typed access to it.

## Internal For Now

Git orchestration, untracked policy implementation, snapshot implementation, and
error internals live behind private modules. Callers should use the crate-root
exports instead of module paths.

## Review Trigger

Revisit this document before M1 exposes registry, recovery, or agent APIs.

