# DevRelay Local RPC Compatibility Policy

Last updated: 2026-06-24

DevRelay local agent RPC uses JSON-RPC 2.0 over the local IPC transport. The
transport is local-machine only; compatibility rules here cover the JSON
request, response, method, and error contract.

Local RPC is not the M4.5 remote Control API. ADR 0005 selects JSON-RPC 2.0
over mTLS for the remote Control API, but the remote server, method allowlist,
auth boundary, schemas, and integration tests are still unimplemented.

## Transport Scope

- macOS/Linux use Unix domain sockets.
- Unix transports must validate local peer credentials where available.
- Windows named pipe transport is not implemented yet.
- Windows per-user pipe ACL is not implemented yet.
- UI may use local RPC only after the active platform's transport enforces the
  local-user boundary.

## Version Negotiation

- Clients must call `rpc.negotiate` before depending on method availability.
- The current protocol version is `1`.
- A server must reject unsupported client protocol versions with
  `RPC_VERSION_MISMATCH` and include both client and server versions.
- A successful negotiation response lists supported method names.

## Request IDs

- Every request must include an ID.
- IDs may be strings or non-negative integers.
- Notifications are not supported in M2.
- Servers must echo the request ID unchanged in success and method-level error
  responses.
- Envelope parse and validation failures return a JSON-RPC error response with
  `id: null`.

## Additive Changes

- New methods may be added without increasing the protocol version.
- New optional params may be added without increasing the protocol version.
- New result fields may be added without increasing the protocol version.
- Clients must ignore unknown result fields.
- `environment.status` is an additive local method in protocol version `1`; it
  reports registered project/workspace hydration state and treats missing state
  files as `cold`/`persisted: false`.
- `metrics.export` is an additive local method in protocol version `1`; it
  writes a redacted, local-only aggregate report and does not include source
  code, snapshot objects, or raw logs.

## Breaking Changes

A protocol version increase is required before:

- Removing or renaming a method.
- Removing a request param that existing clients may send.
- Making an optional param required.
- Removing or changing the type of an existing result field.
- Changing error code meaning.

## Error Contract

- Standard JSON-RPC error codes are used for parse, request, method, params, and
  internal errors.
- DevRelay-specific server errors use the reserved server-error range.
- Error `message` values are stable summaries, while `data.detail` is diagnostic
  text and may change.

## Event Stream Compatibility

- Event streams use monotonic event sequence numbers.
- Clients must reconnect with the last observed cursor.
- A detected gap requires a fresh state load before the UI presents state as
  current.
- New event types may be added without a protocol version bump.
- Clients must ignore unknown event payload fields.
