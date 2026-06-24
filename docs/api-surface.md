# DevRelay API Surface

Last reviewed: 2026-06-24

This document records which DevRelay interfaces are stable enough for other
repo components to depend on. It intentionally separates product contracts from
Rust implementation exports.

## Stability Levels

### Product-Stable

These contracts should not change without a compatibility note, migration plan,
or version bump:

- CLI command names and documented flags in [cli.md](cli.md).
- CLI JSON success output for commands that advertise `--json`.
- CLI JSON error envelope and stable `DR-*` error namespaces.
- Snapshot metadata schema version and persisted field names.
- Local JSON-RPC envelope rules in [rpc-compatibility.md](rpc-compatibility.md).
- Local JSON-RPC method names exposed by the agent.
- Event envelope schema version, sequence behavior, and event type names.
- SQLite migration forward-only behavior.
- Safety policies in [data-loss-safety.md](data-loss-safety.md).

### Repo-Internal Stable

These are stable for the CLI, agent, and future UI inside this repository, but
are not promised as a third-party SDK:

- `devrelay-core` crate-root exports used by `devrelay-cli` and
  `devrelay-agent`.
- Manifest typed structs and parser.
- `GitRepo` status collection.
- snapshot create/apply/verify helpers.
- project registry and workspace mapping types.
- snapshot store and metadata DB helpers.
- lease, session, handoff, and event types.
- CAS, sidecar, data-plane, and route-selection types.
- environment profile selection and adapter reports.
- platform/path/line-ending/WSL doctor reports.

Downstream code inside this workspace should import these from the crate root
instead of private module paths.

### Internal Implementation Details

These may change freely as long as the product-stable contracts remain true:

- exact Git command composition
- temporary index layout
- private module boundaries
- SQLite table internals not exposed by migrations
- local storage directory internals not documented as user-facing
- log record implementation details beyond redaction expectations
- watcher backend implementation details
- retention planner internals
- CAS chunking strategy while manifest compatibility is preserved

## Control API Gap

M4.5 is not implemented. There is no stable remote HTTP `/v1` Control API yet.
The project must choose one of these before M4 can close:

1. Implement the documented HTTP/RPC server over mTLS with auth middleware,
   endpoint schemas, and integration tests.
2. Rename M4.5 to a versioned remote JSON-RPC boundary over mTLS and document
   the method set, auth model, and schema compatibility rules.

Until that decision is complete, do not claim that a remote Control API rejects
unauthenticated requests. The mTLS transport primitives reject invalid peers,
but that is not the same as an implemented API boundary.

## Local Agent Handoff RPC

The local agent JSON-RPC surface now exposes `leases.list` plus the handoff
state-machine methods `handoffs.list`, `handoff.begin`,
`handoff.target.verify`, `handoff.source.ready`, `handoff.commit`,
`handoff.abort`, and `handoff.recover`.

These methods are local metadata-control commands. They do not implement the
remote M4.5 Control API, advertise file transfer completion, or allow UI clients
to decide handoff success before target apply and verification have completed.
`handoff.begin` requires a registered target device and uses the agent's local
device identity as the source device.

## UI Boundary

Production UI may depend only on:

- initial agent RPC state
- agent event subscription
- command results from the agent
- local presentation state that does not compute canonical DevRelay truth

The UI must not read Git directly, scan workspaces directly, infer writer
authority locally, or decide handoff success before agent verification.

## Review Triggers

Review this document before:

- adding a new CLI command used by UI or scripts
- changing any `--json` output
- adding or renaming an agent RPC method
- changing event payload fields
- implementing M4.5 Control API
- starting the Tauri UI
- freezing a release candidate schema
