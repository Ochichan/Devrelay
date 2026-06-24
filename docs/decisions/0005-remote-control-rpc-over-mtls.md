# ADR 0005: Remote Control API Uses Versioned JSON-RPC Over mTLS

Date: 2026-06-24

## Status

Accepted

## Context

M4.5 left the remote Control API shape undecided: either HTTP `/v1`
endpoints or a versioned remote RPC boundary over mTLS. The repository already
has:

- local JSON-RPC envelope compatibility rules,
- agent RPC method naming and request ID behavior,
- event schemas,
- mTLS transport primitives,
- protocol version negotiation,
- replay nonce and clock-skew checks,
- device certificate validation and revocation checks.

Adding a separate REST surface would require a second schema, error, and auth
model for the same control operations.

## Decision

The M4.5 remote Control API will be a versioned JSON-RPC 2.0 boundary over the
existing mTLS control transport. It is not an HTTP `/v1` REST API.

The remote boundary must remain distinct from local IPC:

- mTLS is mandatory.
- Device certificates must chain to the pinned fabric root.
- Revoked devices must be rejected before method dispatch.
- Requests must pass protocol version, timestamp, replay nonce, and request ID
  validation.
- Remote method compatibility must be documented before beta.
- The schema documentation should list params/result JSON for each supported
  remote method.

The first remote method set replaces the old HTTP endpoint placeholders:

- `devices.list`
- `projects.list`
- `workspaces.list`
- `sessions.snapshots.list`
- `handoffs.list`
- `handoff.begin`
- `handoff.abort`
- `handoff.target.verify`
- `handoff.source.ready`
- `handoff.commit`
- `handoff.recover`
- `recovery.list`
- `recovery.open`

## Consequences

The project does not need OpenAPI for M4.5, but it still needs schema
documentation and integration tests for the remote RPC boundary. Local RPC can
keep evolving under `docs/rpc-compatibility.md`; remote RPC needs its own
explicit method allowlist and auth checks before M4 can close.

Until the remote RPC server is implemented, DevRelay must not claim that the
Control API rejects unauthenticated requests through an implemented API
boundary. Existing mTLS primitives are necessary but not sufficient.
